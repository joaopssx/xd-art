//! Recipe files: every setting of a conversion as small, readable JSON.
//!
//! ```json
//! {
//!   "xd-art": 1,
//!   "sample": { "mode": "nearest", "width": 64, "height": 64 },
//!   "colors": { "palette": "pico-8", "metric": "redmean" },
//!   "dither": { "mode": "bayer4x4", "amount": 0.2 }
//! }
//! ```
//!
//! Only settings that differ from the defaults are written. When reading, missing keys
//! take the default and unknown keys are ignored, so a recipe written by a newer
//! version still loads in an older one (minus what the older one does not know), and
//! a recipe stays valid when a later version adds settings. The `"xd-art"` number only
//! goes up when an existing key changes meaning; a recipe with a higher number than
//! this build knows is rejected rather than misread.
//!
//! The JSON presets of the original program can be read too ([`from_original`]).

use serde_json::{Map, Value, json};

use crate::settings::{DitherMode, Metric, PaletteSource, Quantizer, SampleMode, Settings, Size};
use crate::{Error, Result};

/// Format version written by this build.
pub const VERSION: u64 = 1;

fn bad(reason: impl Into<String>) -> Error {
    Error::Invalid {
        name: "recipe",
        reason: reason.into(),
    }
}

#[allow(clippy::too_many_lines)] // one block per settings section, flat on purpose
/// Writes `s` as a recipe. The mask is not part of a recipe: it is an image, and
/// recipes are meant to apply to any image.
#[must_use]
pub fn to_json(s: &Settings) -> String {
    let d = Settings::default();
    let mut root = Map::new();
    root.insert("xd-art".into(), json!(VERSION));
    let mut put = |section: &str, key: &str, v: Value| {
        let e = root
            .entry(section)
            .or_insert_with(|| Value::Object(Map::new()));
        if let Value::Object(m) = e {
            m.insert(key.into(), v);
        }
    };
    if s.blur != d.blur {
        put("blur", "radius", json!(s.blur.radius));
        put("blur", "compatible", json!(s.blur.box_approx));
        put("blur", "box_from", json!(s.blur.box_from));
    }
    if s.sample != d.sample {
        put("sample", "mode", json!(s.sample.mode.name()));
        match s.sample.size {
            Size::Absolute {
                width,
                height,
                keep_aspect,
            } => {
                put("sample", "width", json!(width));
                if keep_aspect {
                    put("sample", "keep_aspect", json!(true));
                } else {
                    put("sample", "height", json!(height));
                }
            }
            Size::Relative { x, y } => {
                put("sample", "divide_x", json!(x));
                put("sample", "divide_y", json!(y));
            }
        }
        if s.sample.offset_x != 0.0 || s.sample.offset_y != 0.0 {
            put(
                "sample",
                "offset",
                json!([s.sample.offset_x, s.sample.offset_y]),
            );
        }
        if s.sample.mode == SampleMode::Edge {
            put("sample", "edge_strength", json!(s.sample.edge_strength));
        }
    }
    let (a, da) = (&s.adjust, &d.adjust);
    for (key, v, dv) in [
        ("sharpen", a.sharpen, da.sharpen),
        ("brightness", a.brightness, da.brightness),
        ("contrast", a.contrast, da.contrast),
        ("saturation", a.saturation, da.saturation),
        ("hue", a.hue, da.hue),
        ("gamma", a.gamma, da.gamma),
        ("tint_strength", a.tint_strength, da.tint_strength),
    ] {
        if v != dv {
            put("adjust", key, json!(v));
        }
    }
    if a.tint != da.tint {
        put("adjust", "tint", json!(hex(a.tint)));
    }
    if s.colors != d.colors {
        match &s.colors.palette {
            PaletteSource::Builtin(name) => put("colors", "palette", json!(name)),
            PaletteSource::Colors(c) => put(
                "colors",
                "palette",
                json!(c.iter().map(|c| hex(*c)).collect::<Vec<_>>()),
            ),
            PaletteSource::Extract {
                count,
                method,
                locked,
            } => {
                let mut m = Map::new();
                m.insert("count".into(), json!(count));
                m.insert("method".into(), json!(quantizer_name(*method)));
                if !locked.is_empty() {
                    m.insert(
                        "locked".into(),
                        json!(locked.iter().map(|c| hex(*c)).collect::<Vec<_>>()),
                    );
                }
                put("colors", "extract", Value::Object(m));
            }
        }
        put("colors", "metric", json!(s.colors.metric.name()));
    }
    if s.dither != d.dither {
        put("dither", "mode", json!(s.dither.mode.name()));
        put("dither", "amount", json!(s.dither.amount));
        put("dither", "alpha_threshold", json!(s.dither.alpha_threshold));
        put("dither", "target_colors", json!(s.dither.target_colors));
    }
    if s.post != d.post {
        if let Some(c) = s.post.outline {
            put("post", "outline", json!(c));
        }
        if let Some(c) = s.post.inline {
            put("post", "inline", json!(c));
        }
        put("post", "cleanup", json!(s.post.cleanup));
        put("post", "merge", json!(s.post.merge));
    }
    if s.output != d.output {
        put("output", "scale", json!(s.output.scale));
    }
    serde_json::to_string_pretty(&Value::Object(root)).unwrap_or_default()
}

fn hex(c: [u8; 3]) -> String {
    format!("{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn unhex(v: &Value) -> Result<[u8; 3]> {
    let s = v.as_str().ok_or_else(|| bad("colors are hex strings"))?;
    let s = s.trim_start_matches('#');
    let n = u32::from_str_radix(s, 16)
        .ok()
        .filter(|_| s.len() == 6)
        .ok_or_else(|| bad(format!("{s} is not a color")))?;
    Ok([(n >> 16) as u8, (n >> 8) as u8, n as u8])
}

fn quantizer_name(q: Quantizer) -> &'static str {
    match q {
        Quantizer::KMeans => "k-means",
        Quantizer::MedianCut => "median-cut",
        Quantizer::Balanced => "balanced",
    }
}

struct Reader<'a>(&'a Map<String, Value>);

impl Reader<'_> {
    fn section(&self, name: &str) -> Option<&Map<String, Value>> {
        self.0.get(name).and_then(Value::as_object)
    }
}

fn f(m: &Map<String, Value>, k: &str, into: &mut f32) -> Result<()> {
    if let Some(v) = m.get(k) {
        *into = v
            .as_f64()
            .ok_or_else(|| bad(format!("{k} must be a number")))? as f32;
    }
    Ok(())
}

fn u(m: &Map<String, Value>, k: &str) -> Result<Option<u64>> {
    m.get(k)
        .map(|v| {
            v.as_u64()
                .ok_or_else(|| bad(format!("{k} must be a whole number")))
        })
        .transpose()
}

fn name<'a>(m: &'a Map<String, Value>, k: &str) -> Result<Option<&'a str>> {
    m.get(k)
        .map(|v| v.as_str().ok_or_else(|| bad(format!("{k} must be a name"))))
        .transpose()
}

/// Reads a recipe.
///
/// # Errors
///
/// Fails on invalid JSON, a newer format version, wrong value types, unknown mode names,
/// or values out of range.
#[allow(clippy::too_many_lines)] // one block per settings section, flat on purpose
pub fn from_json(text: &str) -> Result<Settings> {
    let root: Value = serde_json::from_str(text).map_err(|e| bad(e.to_string()))?;
    let root = root.as_object().ok_or_else(|| bad("not a JSON object"))?;
    let version = root
        .get("xd-art")
        .and_then(Value::as_u64)
        .ok_or_else(|| bad("missing \"xd-art\" version"))?;
    if version > VERSION {
        return Err(bad(format!(
            "version {version} is newer than this build understands ({VERSION})"
        )));
    }
    let r = Reader(root);
    let mut s = Settings::default();
    if let Some(m) = r.section("blur") {
        f(m, "radius", &mut s.blur.radius)?;
        f(m, "box_from", &mut s.blur.box_from)?;
        s.blur.box_approx = m
            .get("compatible")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    }
    if let Some(m) = r.section("sample") {
        if let Some(n) = name(m, "mode")? {
            s.sample.mode = SampleMode::from_name(n)
                .ok_or_else(|| bad(format!("unknown sampling mode {n}")))?;
        }
        let keep = m
            .get("keep_aspect")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if let (Some(x), Some(y)) = (u(m, "divide_x")?, u(m, "divide_y")?) {
            s.sample.size = Size::Relative {
                x: x as u32,
                y: y as u32,
            };
        } else if let Some(w) = u(m, "width")? {
            let h = u(m, "height")?.unwrap_or(if keep { 0 } else { w });
            s.sample.size = Size::Absolute {
                width: w as u32,
                height: h as u32,
                keep_aspect: keep,
            };
        }
        if let Some(o) = m.get("offset").and_then(Value::as_array) {
            s.sample.offset_x = o.first().and_then(Value::as_f64).unwrap_or(0.0) as f32;
            s.sample.offset_y = o.get(1).and_then(Value::as_f64).unwrap_or(0.0) as f32;
        }
        f(m, "edge_strength", &mut s.sample.edge_strength)?;
    }
    if let Some(m) = r.section("adjust") {
        let a = &mut s.adjust;
        f(m, "sharpen", &mut a.sharpen)?;
        f(m, "brightness", &mut a.brightness)?;
        f(m, "contrast", &mut a.contrast)?;
        f(m, "saturation", &mut a.saturation)?;
        f(m, "hue", &mut a.hue)?;
        f(m, "gamma", &mut a.gamma)?;
        f(m, "tint_strength", &mut a.tint_strength)?;
        if let Some(t) = m.get("tint") {
            a.tint = unhex(t)?;
        }
    }
    if let Some(m) = r.section("colors") {
        match m.get("palette") {
            Some(Value::String(n)) => s.colors.palette = PaletteSource::Builtin(n.clone()),
            Some(Value::Array(c)) => {
                s.colors.palette =
                    PaletteSource::Colors(c.iter().map(unhex).collect::<Result<_>>()?);
            }
            Some(_) => return Err(bad("palette must be a name or a list of colors")),
            None => {}
        }
        if let Some(e) = m.get("extract").and_then(Value::as_object) {
            let count = u(e, "count")?.unwrap_or(16) as u16;
            let method = match name(e, "method")?.unwrap_or("k-means") {
                "k-means" => Quantizer::KMeans,
                "median-cut" => Quantizer::MedianCut,
                "balanced" => Quantizer::Balanced,
                other => return Err(bad(format!("unknown palette method {other}"))),
            };
            let locked = match e.get("locked").and_then(Value::as_array) {
                Some(l) => l.iter().map(unhex).collect::<Result<_>>()?,
                None => Vec::new(),
            };
            s.colors.palette = PaletteSource::Extract {
                count,
                method,
                locked,
            };
        }
        if let Some(n) = name(m, "metric")? {
            s.colors.metric =
                Metric::from_name(n).ok_or_else(|| bad(format!("unknown metric {n}")))?;
        }
    }
    if let Some(m) = r.section("dither") {
        if let Some(n) = name(m, "mode")? {
            s.dither.mode =
                DitherMode::from_name(n).ok_or_else(|| bad(format!("unknown dither mode {n}")))?;
        }
        f(m, "amount", &mut s.dither.amount)?;
        if let Some(v) = u(m, "alpha_threshold")? {
            s.dither.alpha_threshold = v.min(255) as u8;
        }
        if let Some(v) = u(m, "target_colors")? {
            s.dither.target_colors = v.min(256) as u16;
        }
    }
    if let Some(m) = r.section("post") {
        s.post.outline = u(m, "outline")?.map(|v| v.min(255) as u8);
        s.post.inline = u(m, "inline")?.map(|v| v.min(255) as u8);
        s.post.cleanup = u(m, "cleanup")?.unwrap_or(0).min(255) as u8;
        s.post.merge = u(m, "merge")?.unwrap_or(0).min(255) as u8;
    }
    if let Some(m) = r.section("output")
        && let Some(v) = u(m, "scale")?
    {
        s.output.scale = v as u32;
    }
    s.validate()?;
    Ok(s)
}

/// Dither modes in the order of the original's enum, which its presets store.
const ORIGINAL_DITHER: [DitherMode; 17] = [
    DitherMode::None,
    DitherMode::Bayer8,
    DitherMode::Bayer4,
    DitherMode::Bayer2,
    DitherMode::Cluster8,
    DitherMode::Cluster4,
    DitherMode::Floyd,
    DitherMode::FloydMono,
    DitherMode::MedianCut,
    DitherMode::Bayer5,
    DitherMode::Bayer3,
    DitherMode::Stucki,
    DitherMode::Burkes,
    DitherMode::Sierra,
    DitherMode::SierraTwoRow,
    DitherMode::SierraLite,
    DitherMode::Picocad,
];

/// Reads a preset saved by the original program's interface (its "Save preset" JSON).
///
/// # Errors
///
/// Fails on invalid JSON or out of range values.
#[allow(clippy::too_many_lines)] // one line per key of the original format
pub fn from_original(text: &str) -> Result<Settings> {
    let root: Value = serde_json::from_str(text).map_err(|e| bad(e.to_string()))?;
    let m = root.as_object().ok_or_else(|| bad("not a JSON object"))?;
    let num = |k: &str, d: f64| m.get(k).and_then(Value::as_f64).unwrap_or(d);
    let int = |k: &str, d: i64| m.get(k).and_then(Value::as_i64).unwrap_or(d);
    let mut s = Settings::default();
    s.blur.radius = num("blur_amount", 0.0) as f32;
    // The original only has its own box blur.
    s.blur.box_approx = true;
    s.sample.mode = match int("sample_mode", 0) {
        1 => SampleMode::Bilinear,
        2 => SampleMode::Bicubic,
        3 => SampleMode::Lanczos,
        4 => SampleMode::Cluster,
        _ => SampleMode::Nearest,
    };
    s.sample.offset_x = num("x_offset", 0.0) as f32;
    s.sample.offset_y = num("y_offset", 0.0) as f32;
    s.sample.size = if m
        .get("scale_relative")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        Size::Relative {
            x: int("size_relative_x", 2).max(1) as u32,
            y: int("size_relative_y", 2).max(1) as u32,
        }
    } else {
        Size::Absolute {
            width: int("size_absolute_x", 64).max(1) as u32,
            height: int("size_absolute_y", 64).max(1) as u32,
            keep_aspect: false,
        }
    };
    s.adjust.sharpen = num("sharp_amount", 0.0) as f32;
    s.adjust.brightness = num("brightness", 0.0) as f32;
    s.adjust.contrast = num("contrast", 1.0) as f32;
    s.adjust.saturation = num("saturation", 1.0) as f32;
    s.adjust.hue = num("hue", 0.0) as f32;
    s.adjust.gamma = num("gamma", 1.0) as f32;
    s.adjust.tint = [
        int("tint_red", 255),
        int("tint_green", 255),
        int("tint_blue", 255),
    ]
    .map(|v| v.clamp(0, 255) as u8);
    s.dither.alpha_threshold = int("dither_alpha_threshold", 128).clamp(0, 255) as u8;
    s.dither.amount = num("dither_dither_amount", 0.2) as f32;
    s.dither.target_colors = int("dither_target_colors", 8).clamp(1, 256) as u16;
    s.dither.mode = usize::try_from(int("dither_dither_mode", 0))
        .ok()
        .and_then(|i| ORIGINAL_DITHER.get(i))
        .copied()
        .unwrap_or_default();
    s.colors.metric = usize::try_from(int("dither_color_dist", 2))
        .ok()
        .and_then(|i| Metric::ALL.get(i))
        .copied()
        .unwrap_or_default();
    let count = usize::try_from(int("dither_palette_colors", 0))
        .unwrap_or(0)
        .min(256);
    if let Some(p) = m.get("dither_palette").and_then(Value::as_array)
        && count > 0
    {
        // Colors are stored as 0xAABBGGRR integers.
        let colors: Vec<[u8; 3]> = p
            .iter()
            .take(count)
            .filter_map(Value::as_u64)
            .map(|c| [c as u8, (c >> 8) as u8, (c >> 16) as u8])
            .collect();
        if !colors.is_empty() {
            s.colors.palette = PaletteSource::Colors(colors);
        }
    }
    s.validate()?;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_write_only_the_version() {
        let text = to_json(&Settings::default());
        assert_eq!(text.replace(char::is_whitespace, ""), "{\"xd-art\":1}");
        assert_eq!(from_json(&text).unwrap(), Settings::default());
    }

    #[test]
    fn everything_round_trips() {
        let mut s = Settings::default();
        s.blur.radius = 1.5;
        s.blur.box_approx = true;
        s.sample.mode = SampleMode::Edge;
        s.sample.size = Size::Absolute {
            width: 48,
            height: 0,
            keep_aspect: true,
        };
        s.sample.offset_x = 0.25;
        s.adjust.gamma = 1.2;
        s.adjust.tint = [255, 200, 180];
        s.colors.palette = PaletteSource::Extract {
            count: 12,
            method: Quantizer::Balanced,
            locked: vec![[1, 2, 3]],
        };
        s.colors.metric = Metric::Ciede2000;
        s.dither.mode = DitherMode::SierraLite;
        s.post.outline = Some(0);
        s.post.cleanup = 2;
        s.output.scale = 4;
        let back = from_json(&to_json(&s)).unwrap();
        assert_eq!(back, s);
        let mut r = s.clone();
        r.sample.size = Size::Relative { x: 3, y: 4 };
        r.colors.palette = PaletteSource::Colors(vec![[0; 3], [255, 0, 128]]);
        assert_eq!(from_json(&to_json(&r)).unwrap(), r);
    }

    #[test]
    fn unknown_keys_are_ignored_and_newer_versions_rejected() {
        let s = from_json(
            r#"{"xd-art": 1, "future": {"x": 1}, "dither": {"mode": "floyd", "new_key": true}}"#,
        )
        .unwrap();
        assert_eq!(s.dither.mode, DitherMode::Floyd);
        let e = from_json(r#"{"xd-art": 2}"#).unwrap_err().to_string();
        assert!(e.contains("newer"), "{e}");
        assert!(from_json("{}").is_err());
        assert!(from_json(r#"{"xd-art": 1, "dither": {"mode": "wobbly"}}"#).is_err());
        assert!(from_json(r#"{"xd-art": 1, "output": {"scale": 0}}"#).is_err());
    }

    #[test]
    fn original_presets_load() {
        let text = r#"{"blur_amount":1.5,"sample_mode":4,"x_offset":0.0,"y_offset":0.1,
            "scale_relative":false,"size_relative_x":2,"size_relative_y":2,
            "size_absolute_x":80,"size_absolute_y":60,"sharp_amount":0.3,"brightness":0.0,
            "contrast":1.0,"saturation":1.2,"hue":0.0,"gamma":1.0,"kmeanspp":false,
            "dither_alpha_threshold":128,"dither_dither_amount":0.25,"dither_target_colors":8,
            "dither_dither_mode":2,"dither_color_dist":5,"tint_red":255,"tint_green":255,
            "tint_blue":255,"dither_palette_colors":2,"dither_palette":[4278190080,4294967295,0]}"#;
        let s = from_original(text).unwrap();
        assert_eq!(s.sample.mode, SampleMode::Cluster);
        assert_eq!(
            s.sample.size,
            Size::Absolute {
                width: 80,
                height: 60,
                keep_aspect: false
            }
        );
        assert_eq!(s.dither.mode, DitherMode::Bayer4);
        assert_eq!(s.colors.metric, Metric::Ciede2000);
        assert_eq!(
            s.colors.palette,
            PaletteSource::Colors(vec![[0, 0, 0], [255, 255, 255]])
        );
        assert!(s.blur.box_approx);
    }
}
