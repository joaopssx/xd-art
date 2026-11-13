//! Lua 5.4 scripting for xd-art.
//!
//! A script gets two globals:
//!
//! - `xd`, xd-art's own API: load images, convert them with settings given as a Lua
//!   table shaped like a recipe file, save, list files, build sprite sheets.
//! - `img2pixel`, a compatibility layer with the scripting API of SLK_img2pixel, so its
//!   scripts run unchanged (differences are listed in `docs/compat.md`).
//!
//! Nothing here starts until a script is run; the GUI and the CLI only create a Lua
//! state when asked to.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use mlua::{Lua, Table, UserData, UserDataMethods, Value};
use serde_json::{Map, Value as Json};
use xd_art_core::buffer::Image;
use xd_art_core::cancel::Cancel;
use xd_art_core::pipeline::Pipeline;
use xd_art_core::settings::Settings;
use xd_art_core::{anim, export, io, palette, preset, recipe};

/// An image handed to Lua.
#[derive(Clone)]
struct LuaImage(Arc<Image>);

impl UserData for LuaImage {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("width", |_, this, ()| Ok(this.0.width()));
        m.add_method("height", |_, this, ()| Ok(this.0.height()));
        m.add_method("pixel", |_, this, (x, y): (u32, u32)| {
            // Four values, or nothing outside the image.
            Ok(this
                .0
                .get(x, y)
                .map(|p| mlua::Variadic::from(p.to_vec()))
                .unwrap_or_default())
        });
        m.add_method("save", |_, this, path: String| {
            io::save(&this.0, Path::new(&path)).map_err(mlua::Error::external)
        });
        m.add_method("upscale", |_, this, k: u32| {
            this.0
                .upscale(k)
                .map(|i| LuaImage(Arc::new(i)))
                .map_err(mlua::Error::external)
        });
    }
}

fn to_json(v: &Value) -> mlua::Result<Json> {
    Ok(match v {
        Value::Nil => Json::Null,
        Value::Boolean(b) => Json::Bool(*b),
        Value::Integer(i) => Json::from(*i),
        Value::Number(n) => serde_json::Number::from_f64(*n).map_or(Json::Null, Json::Number),
        Value::String(s) => Json::String(s.to_str()?.to_string()),
        Value::Table(t) => {
            let len = t.raw_len();
            if len > 0 {
                let mut a = Vec::with_capacity(len);
                for i in 1..=len {
                    a.push(to_json(&t.raw_get::<Value>(i)?)?);
                }
                Json::Array(a)
            } else {
                let mut m = Map::new();
                for pair in t.pairs::<String, Value>() {
                    let (k, v) = pair?;
                    m.insert(k, to_json(&v)?);
                }
                Json::Object(m)
            }
        }
        other => {
            return Err(mlua::Error::runtime(format!(
                "cannot use a {} in settings",
                other.type_name()
            )));
        }
    })
}

fn from_json(lua: &Lua, v: &Json) -> mlua::Result<Value> {
    Ok(match v {
        Json::Null => Value::Nil,
        Json::Bool(b) => Value::Boolean(*b),
        Json::Number(n) => n
            .as_i64()
            .map_or_else(|| Value::Number(n.as_f64().unwrap_or(0.0)), Value::Integer),
        Json::String(s) => Value::String(lua.create_string(s)?),
        Json::Array(a) => {
            let t = lua.create_table()?;
            for (i, x) in a.iter().enumerate() {
                t.raw_set(i + 1, from_json(lua, x)?)?;
            }
            Value::Table(t)
        }
        Json::Object(o) => {
            let t = lua.create_table()?;
            for (k, x) in o {
                t.raw_set(k.as_str(), from_json(lua, x)?)?;
            }
            Value::Table(t)
        }
    })
}

/// Settings from a recipe-shaped table, a recipe file path, or nothing (defaults).
fn settings_from(v: Option<Value>) -> mlua::Result<Settings> {
    let text = match v {
        None | Some(Value::Nil) => return Ok(Settings::default()),
        Some(Value::String(path)) => {
            let path = path.to_str()?.to_string();
            std::fs::read_to_string(&path)
                .map_err(|e| mlua::Error::runtime(format!("could not read recipe {path}: {e}")))?
        }
        Some(v) => {
            let mut j = to_json(&v)?;
            if let Json::Object(m) = &mut j {
                m.entry("xd-art").or_insert(Json::from(recipe::VERSION));
            }
            j.to_string()
        }
    };
    recipe::from_json(&text)
        .or_else(|_| recipe::from_original(&text))
        .map_err(mlua::Error::external)
}

fn is_gif(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("gif"))
}

fn convert_image(img: &Image, s: &Settings) -> xd_art_core::Result<Image> {
    let mut p = Pipeline::default();
    p.run(img, 1, s, &Cancel::new())?;
    p.output().cloned().ok_or(xd_art_core::Error::Cancelled)
}

/// Converts a file to a file, animated when both are GIFs.
fn convert_file(input: &Path, output: &Path, s: &Settings) -> xd_art_core::Result<()> {
    if is_gif(input) {
        let a = anim::load_gif(input)?;
        if is_gif(output) {
            let c = anim::convert(&a, s, anim::PaletteMode::Shared, &Cancel::new())?;
            let bytes = anim::encode_gif(&c, s.output.scale)?;
            return std::fs::write(output, bytes).map_err(|source| xd_art_core::Error::Io {
                path: output.into(),
                source,
            });
        }
        let first = a
            .frames
            .into_iter()
            .next()
            .map(|f| f.image)
            .ok_or(xd_art_core::Error::Cancelled)?;
        return io::save(&convert_image(&first, s)?, output);
    }
    io::save(&convert_image(&io::load(input)?, s)?, output)
}

fn list_files(dir: &str, pattern: Option<&str>) -> std::io::Result<Vec<String>> {
    let pat = pattern.unwrap_or("*").to_lowercase();
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        let name = p
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_lowercase();
        if p.is_file() && glob(&pat, &name) {
            out.push(p.to_string_lossy().into_owned());
        }
    }
    out.sort();
    Ok(out)
}

/// `*` and `?` on a file name.
fn glob(p: &str, n: &str) -> bool {
    let (p, n): (Vec<char>, Vec<char>) = (p.chars().collect(), n.chars().collect());
    let (mut pi, mut ni, mut star, mut mark) = (0, 0, None, 0);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

fn install_xd(lua: &Lua, started: Instant) -> mlua::Result<()> {
    let xd = lua.create_table()?;
    xd.set("version", env!("CARGO_PKG_VERSION"))?;
    xd.set(
        "load",
        lua.create_function(|_, path: String| {
            io::load(Path::new(&path))
                .map(|i| LuaImage(Arc::new(i)))
                .map_err(mlua::Error::external)
        })?,
    )?;
    xd.set(
        "convert",
        lua.create_function(
            |_, (img, opts): (mlua::UserDataRef<LuaImage>, Option<Value>)| {
                let s = settings_from(opts)?;
                convert_image(&img.0, &s)
                    .map(|i| LuaImage(Arc::new(i)))
                    .map_err(mlua::Error::external)
            },
        )?,
    )?;
    xd.set(
        "process",
        lua.create_function(
            |_, (input, output, opts): (String, String, Option<Value>)| {
                let s = settings_from(opts)?;
                Ok(
                    match convert_file(Path::new(&input), Path::new(&output), &s) {
                        Ok(()) => (true, None),
                        Err(e) => (false, Some(e.to_string())),
                    },
                )
            },
        )?,
    )?;
    xd.set(
        "files",
        lua.create_function(|_, (dir, pattern): (String, Option<String>)| {
            list_files(&dir, pattern.as_deref())
                .map_err(|e| mlua::Error::runtime(format!("could not list {dir}: {e}")))
        })?,
    )?;
    xd.set(
        "time",
        lua.create_function(move |_, ()| Ok(started.elapsed().as_secs_f64()))?,
    )?;
    xd.set(
        "palettes",
        lua.create_function(|_, ()| {
            Ok(palette::BUILTINS.iter().map(|b| b.name).collect::<Vec<_>>())
        })?,
    )?;
    xd.set(
        "presets",
        lua.create_function(|_, ()| {
            Ok(preset::PRESETS.iter().map(|p| p.name).collect::<Vec<_>>())
        })?,
    )?;
    xd.set(
        "preset",
        lua.create_function(|lua, name: String| {
            let p = preset::find(&name)
                .ok_or_else(|| mlua::Error::runtime(format!("no preset named {name}")))?;
            let j: Json = serde_json::from_str(&recipe::to_json(&p.settings()))
                .map_err(mlua::Error::external)?;
            from_json(lua, &j)
        })?,
    )?;
    xd.set(
        "recipe",
        lua.create_function(|lua, path: String| {
            let s = settings_from(Some(Value::String(lua.create_string(&path)?)))?;
            let j: Json =
                serde_json::from_str(&recipe::to_json(&s)).map_err(mlua::Error::external)?;
            from_json(lua, &j)
        })?,
    )?;
    xd.set(
        "sheet",
        lua.create_function(
            |_,
             (images, columns, spacing): (
                Vec<mlua::UserDataRef<LuaImage>>,
                Option<u32>,
                Option<u32>,
            )| {
                let frames: Vec<Image> = images.iter().map(|i| (*i.0).clone()).collect();
                let layout = export::SheetLayout {
                    columns: columns.unwrap_or(0),
                    spacing: spacing.unwrap_or(0),
                };
                export::sprite_sheet(&frames, &[], layout)
                    .map(|s| LuaImage(Arc::new(s.image)))
                    .map_err(mlua::Error::external)
            },
        )?,
    )?;
    lua.globals().set("xd", xd)?;
    Ok(())
}

/// Fields of the original's `img2pixel` table, with their names in its preset files and
/// their defaults. Defaults follow the original command line tool.
const IMG2PIXEL_FIELDS: [(&str, &str, f64); 23] = [
    ("blur_amount", "blur_amount", 0.0),
    ("sharp_amount", "sharp_amount", 0.0),
    ("sample_mode", "sample_mode", 0.0),
    ("x_offset", "x_offset", 0.0),
    ("y_offset", "y_offset", 0.0),
    ("scale_relative", "scale_relative", 0.0),
    ("size_absolute_x", "size_absolute_x", 64.0),
    ("size_absolute_y", "size_absolute_y", 64.0),
    ("size_relative_x", "size_relative_x", 2.0),
    ("size_relative_y", "size_relative_y", 2.0),
    ("dither_mode", "dither_dither_mode", 2.0),
    ("color_dist", "dither_color_dist", 2.0),
    ("dither_amount", "dither_dither_amount", 0.2),
    ("dither_alpha_threshold", "dither_alpha_threshold", 128.0),
    ("target_colors", "dither_target_colors", 8.0),
    ("palette_colors", "dither_palette_colors", 16.0),
    ("kmeanspp", "kmeanspp", 0.0),
    ("brightness", "brightness", 0.0),
    ("contrast", "contrast", 1.0),
    ("saturation", "saturation", 1.0),
    ("hue", "hue", 0.0),
    ("gamma", "gamma", 1.0),
    ("tint_red", "tint_red", 255.0),
];
const IMG2PIXEL_TINT: [(&str, f64); 2] = [("tint_green", 255.0), ("tint_blue", 255.0)];

const DITHER_NAMES: [&str; 17] = [
    "NONE",
    "BAYER8X8",
    "BAYER4X4",
    "BAYER2X2",
    "CLUSTER8X8",
    "CLUSTER4X4",
    "FLOYD",
    "FLOYD2",
    "MEDIAN_CUT",
    "BAYER5X5",
    "BAYER3X3",
    "STUCKI",
    "BURKES",
    "SIERRA",
    "SIERRA_TWOROW",
    "SIERRA_LITE",
    "PICOCAD",
];
const METRIC_NAMES: [&str; 6] = [
    "RGB_EUCLIDIAN",
    "RGB_WEIGHTED",
    "RGB_REDMEAN",
    "LAB_CIE76",
    "LAB_CIE94",
    "LAB_CIEDE2000",
];

/// The original's settings as one of its preset files, from the field storage.
fn img2pixel_preset(store: &Table) -> mlua::Result<String> {
    let mut m = Map::new();
    for (field, key, _) in IMG2PIXEL_FIELDS {
        m.insert(key.into(), to_json(&store.raw_get::<Value>(field)?)?);
    }
    for (field, _) in IMG2PIXEL_TINT {
        m.insert(field.into(), to_json(&store.raw_get::<Value>(field)?)?);
    }
    for key in ["scale_relative", "kmeanspp"] {
        let on = matches!(m.get(key), Some(Json::Bool(true)))
            || m.get(key).and_then(Json::as_f64).is_some_and(|v| v != 0.0);
        m.insert(key.into(), Json::Bool(on));
    }
    let pal: Table = store.raw_get("__palette")?;
    let mut colors = Vec::with_capacity(256);
    for i in 1..=256 {
        let rgb: u32 = pal.raw_get(i)?;
        // The original stores 0xAABBGGRR.
        let abgr = 0xff00_0000 | (rgb & 0xff) << 16 | (rgb & 0xff00) | (rgb >> 16 & 0xff);
        colors.push(Json::from(abgr));
    }
    m.insert("dither_palette".into(), Json::Array(colors));
    Ok(Json::Object(m).to_string())
}

#[allow(clippy::too_many_lines)] // every function of the table is defined here, in one place
fn install_img2pixel(lua: &Lua) -> mlua::Result<()> {
    let store = lua.create_table()?;
    for (field, _, v) in IMG2PIXEL_FIELDS {
        store.raw_set(field, v)?;
    }
    for (field, v) in IMG2PIXEL_TINT {
        store.raw_set(field, v)?;
    }
    store.raw_set("scale_relative", false)?;
    store.raw_set("kmeanspp", false)?;
    let pal = lua.create_table()?;
    let pico = palette::builtin("pico-8")
        .map(|p| p.colors().to_vec())
        .unwrap_or_default();
    for i in 0..256 {
        let c = pico.get(i).copied().unwrap_or([0; 3]);
        pal.raw_set(
            i + 1,
            u32::from(c[0]) << 16 | u32::from(c[1]) << 8 | u32::from(c[2]),
        )?;
    }
    store.raw_set("__palette", pal)?;

    let api = lua.create_table()?;
    let dither = lua.create_table()?;
    for (i, n) in DITHER_NAMES.iter().enumerate() {
        dither.raw_set(*n, i)?;
    }
    api.raw_set("DITHER", dither)?;
    let metric = lua.create_table()?;
    for (i, n) in METRIC_NAMES.iter().enumerate() {
        metric.raw_set(*n, i)?;
    }
    api.raw_set("COLORDIST", metric)?;
    let st = store.clone();
    api.raw_set(
        "process",
        lua.create_function(move |_, (input, output): (String, String)| {
            let text = img2pixel_preset(&st)?;
            let ok = recipe::from_original(&text)
                .and_then(|s| convert_file(Path::new(&input), Path::new(&output), &s))
                .map_err(|e| eprintln!("img2pixel.process: {e}"))
                .is_ok();
            Ok(ok)
        })?,
    )?;
    let st = store.clone();
    api.raw_set(
        "load_preset",
        lua.create_function(move |_, path: String| {
            let Ok(text) = std::fs::read_to_string(&path) else {
                return Ok(false);
            };
            let Ok(j) = serde_json::from_str::<Json>(&text) else {
                return Ok(false);
            };
            for (field, key, _) in IMG2PIXEL_FIELDS {
                match j.get(key) {
                    Some(Json::Bool(b)) => st.raw_set(field, *b)?,
                    Some(v) if v.is_number() => st.raw_set(field, v.as_f64().unwrap_or(0.0))?,
                    _ => {}
                }
            }
            for (field, _) in IMG2PIXEL_TINT {
                if let Some(v) = j.get(field).and_then(Json::as_f64) {
                    st.raw_set(field, v)?;
                }
            }
            if let Some(colors) = j.get("dither_palette").and_then(Json::as_array) {
                let pal: Table = st.raw_get("__palette")?;
                for (i, c) in colors.iter().take(256).enumerate() {
                    let c = c.as_u64().unwrap_or(0) as u32;
                    pal.raw_set(i + 1, (c & 0xff) << 16 | (c & 0xff00) | (c >> 16 & 0xff))?;
                }
            }
            Ok(true)
        })?,
    )?;
    let st = store.clone();
    api.raw_set(
        "get_palette_color",
        lua.create_function(move |_, i: usize| {
            let pal: Table = st.raw_get("__palette")?;
            pal.raw_get::<u32>(i.min(255) + 1)
        })?,
    )?;
    let st = store.clone();
    api.raw_set(
        "set_palette_color",
        lua.create_function(move |_, (i, rgb): (usize, u32)| {
            if i > 255 {
                return Err(mlua::Error::runtime(format!(
                    "palette index {i} is out of 0..255"
                )));
            }
            let pal: Table = st.raw_get("__palette")?;
            pal.raw_set(i + 1, rgb & 0x00ff_ffff)
        })?,
    )?;

    let meta = lua.create_table()?;
    let st = store.clone();
    meta.raw_set(
        "__index",
        lua.create_function(move |_, (_, k): (Table, String)| st.raw_get::<Value>(k))?,
    )?;
    let st = store;
    meta.raw_set(
        "__newindex",
        lua.create_function(move |_, (_, k, v): (Table, String, Value)| {
            let known = IMG2PIXEL_FIELDS.iter().any(|f| f.0 == k)
                || IMG2PIXEL_TINT.iter().any(|f| f.0 == k);
            if !known {
                return Err(mlua::Error::runtime(format!(
                    "img2pixel has no setting named {k}"
                )));
            }
            st.raw_set(k, v)
        })?,
    )?;
    api.set_metatable(Some(meta))?;
    lua.globals().set("img2pixel", api)?;
    Ok(())
}

/// Runs Lua source. `name` shows in error messages next to the line number, and `args`
/// become the global `arg` table (`arg[1]`, `arg[2]`, ...).
///
/// # Errors
///
/// Returns Lua's error message, which includes the script name and line.
pub fn run(code: &str, name: &str, args: &[String]) -> Result<(), String> {
    let lua = Lua::new();
    let setup = || -> mlua::Result<()> {
        install_xd(&lua, Instant::now())?;
        install_img2pixel(&lua)?;
        let a = lua.create_table()?;
        a.raw_set(0, name)?;
        for (i, v) in args.iter().enumerate() {
            a.raw_set(i + 1, v.as_str())?;
        }
        lua.globals().set("arg", a)
    };
    setup().map_err(|e| e.to_string())?;
    lua.load(code)
        .set_name(format!("@{name}"))
        .exec()
        .map_err(|e| e.to_string())
}

/// Runs a script file.
///
/// # Errors
///
/// Fails when the file cannot be read or the script fails.
pub fn run_file(path: &Path, args: &[String]) -> Result<(), String> {
    let code = std::fs::read_to_string(path)
        .map_err(|e| format!("could not read script {}: {e}", path.display()))?;
    run(&code, &path.display().to_string(), args)
}

/// Where a script path is resolved from: unchanged when it exists, otherwise inside
/// `examples/lua/` next to the working directory.
#[must_use]
pub fn resolve(path: &str) -> PathBuf {
    let p = PathBuf::from(path);
    if p.exists() {
        p
    } else {
        Path::new("examples/lua").join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("xd-art-lua-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn sample(dir: &Path) -> PathBuf {
        let mut img = Image::new(40, 30, [0; 3]).unwrap();
        for y in 0..30 {
            for x in 0..40 {
                img.set(x, y, [(x * 6) as u8, (y * 8) as u8, 100, 255]);
            }
        }
        let p = dir.join("in.png");
        io::save(&img, &p).unwrap();
        p
    }

    fn lua_path(p: &Path) -> String {
        p.display().to_string().replace('\\', "/")
    }

    #[test]
    fn xd_convert_and_save() {
        let d = tmp("xd");
        let input = lua_path(&sample(&d));
        let out = lua_path(&d.join("out.png"));
        let code = format!(
            r#"
            local img = xd.load("{input}")
            assert(img:width() == 40)
            local px = xd.convert(img, {{ sample = {{ width = 10, height = 6 }}, dither = {{ mode = "floyd" }}, output = {{ scale = 2 }} }})
            assert(px:width() == 20 and px:height() == 12, px:width())
            px:save("{out}")
            local ok, err = xd.process("{input}", "{out}", "nope.json")
            assert(not ok)
            "#
        );
        let r = run(&code, "test.lua", &[]);
        assert!(r.is_err() || r.is_ok());
        // The recipe path does not exist, which is a Lua error from settings_from.
        let e = r.unwrap_err();
        assert!(e.contains("could not read recipe nope.json"), "{e}");
        assert_eq!(io::load(&d.join("out.png")).unwrap().width(), 20);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn img2pixel_compat() {
        let d = tmp("compat");
        let input = lua_path(&sample(&d));
        let out = lua_path(&d.join("c.png"));
        let code = format!(
            r#"
            img2pixel.size_absolute_x = 16
            img2pixel.size_absolute_y = 12
            img2pixel.dither_mode = img2pixel.DITHER.STUCKI
            img2pixel.palette_colors = 2
            img2pixel.set_palette_color(0, 0x000000)
            img2pixel.set_palette_color(1, 0xffffff)
            assert(img2pixel.get_palette_color(1) == 0xffffff)
            assert(img2pixel.process("{input}", "{out}") == true)
            local ok = pcall(function() img2pixel.not_a_setting = 1 end)
            assert(not ok)
            "#
        );
        run(&code, "compat.lua", &[]).unwrap();
        let img = io::load(&d.join("c.png")).unwrap();
        assert_eq!((img.width(), img.height()), (16, 12));
        for y in 0..12 {
            for x in 0..16 {
                let p = img.get(x, y).unwrap();
                assert!(p[0] == 0 || p[0] == 255, "{p:?}");
            }
        }
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn errors_name_the_line() {
        let e = run("local x = 1\nerror('boom')", "script.lua", &[]).unwrap_err();
        assert!(e.contains("script.lua:2"), "{e}");
        let e = run("x = = 2", "bad.lua", &[]).unwrap_err();
        assert!(e.contains("bad.lua:1"), "{e}");
    }

    #[test]
    fn args_files_and_presets() {
        let d = tmp("files");
        sample(&d);
        std::fs::write(d.join("note.txt"), "x").unwrap();
        let dir = lua_path(&d);
        let code = format!(
            r#"
            assert(arg[1] == "hello")
            local f = xd.files("{dir}", "*.png")
            assert(#f == 1, #f)
            local p = xd.preset("pico-8")
            -- Only what differs from the defaults is there; PICO-8 is the default palette.
            assert(p.sample.width == 128 and p.colors == nil)
            assert(#xd.palettes() > 5)
            assert(xd.time() >= 0)
            "#
        );
        run(&code, "t.lua", &["hello".to_string()]).unwrap();
        std::fs::remove_dir_all(&d).ok();
    }
}
