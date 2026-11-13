//! Command line options for `convert` and `batch`, turned into engine settings.

use std::ffi::OsString;
use std::path::PathBuf;

use lexopt::prelude::*;
use xd_art_core::anim::PaletteMode;
use xd_art_core::export::Effects;
use xd_art_core::settings::{
    DitherMode, Metric, PaletteSource, Quantizer, SampleMode, Settings, Size,
};

/// Everything `convert` and `batch` accept besides the input and output.
#[derive(Debug, Default)]
pub struct Options {
    pub settings: Settings,
    pub effects: Effects,
    pub gif_palette: PaletteMode,
    pub save_recipe: Option<PathBuf>,
    pub handoff: Option<PathBuf>,
    pub mask: Option<PathBuf>,
    pub suggest: bool,
    pub json: bool,
    pub quiet: bool,
    pub overwrite: bool,
    pub threads: Option<usize>,
}

/// A usage error: bad flag, bad value. Exit code 2.
#[derive(Debug)]
pub struct Usage(pub String);

impl From<lexopt::Error> for Usage {
    fn from(e: lexopt::Error) -> Self {
        Self(e.to_string())
    }
}

fn hex(s: &str) -> Result<[u8; 3], Usage> {
    let t = s.trim_start_matches('#');
    let v = u32::from_str_radix(t, 16).ok().filter(|_| t.len() == 6);
    v.map(|n| [(n >> 16) as u8, (n >> 8) as u8, n as u8])
        .ok_or_else(|| Usage(format!("{s} is not a color like ff8800")))
}

fn num<T: std::str::FromStr>(name: &str, v: OsString) -> Result<T, Usage> {
    let s = v.string()?;
    s.parse()
        .map_err(|_| Usage(format!("{name}: {s} is not a valid number")))
}

/// Parses one option (given without dashes: `w`, `width`) that belongs to the
/// conversion. Returns `Ok(false)` when it is not one of them, so the caller can handle
/// its own options.
#[allow(clippy::too_many_lines)] // one arm per option; splitting them would only hide the list
pub fn option(key: &str, p: &mut lexopt::Parser, o: &mut Options) -> Result<bool, Usage> {
    let s = &mut o.settings;
    match key {
        "preset" => {
            let name = p.value()?.string()?;
            let preset = xd_art_core::preset::find(&name).ok_or_else(|| {
                Usage(format!(
                    "no preset named {name}, see `xd-art-cli list presets`"
                ))
            })?;
            *s = preset.settings();
        }
        "recipe" => {
            let path = PathBuf::from(p.value()?);
            let text = std::fs::read_to_string(&path)
                .map_err(|e| Usage(format!("could not read recipe {}: {e}", path.display())))?;
            *s = xd_art_core::recipe::from_json(&text)
                .or_else(|_| xd_art_core::recipe::from_original(&text))
                .map_err(|e| Usage(format!("{}: {e}", path.display())))?;
        }
        "save-recipe" => o.save_recipe = Some(p.value()?.into()),
        "w" | "width" => {
            let width = num("width", p.value()?)?;
            s.sample.size = match s.sample.size {
                Size::Absolute {
                    height,
                    keep_aspect,
                    ..
                } => Size::Absolute {
                    width,
                    height,
                    keep_aspect,
                },
                Size::Relative { .. } => Size::Absolute {
                    width,
                    height: width,
                    keep_aspect: false,
                },
            };
        }
        "h" | "height" => {
            let height = num("height", p.value()?)?;
            s.sample.size = match s.sample.size {
                Size::Absolute { width, .. } => Size::Absolute {
                    width,
                    height,
                    keep_aspect: false,
                },
                Size::Relative { .. } => Size::Absolute {
                    width: height,
                    height,
                    keep_aspect: false,
                },
            };
        }
        "keep-aspect" => {
            if let Size::Absolute { width, height, .. } = s.sample.size {
                s.sample.size = Size::Absolute {
                    width,
                    height,
                    keep_aspect: true,
                };
            }
        }
        "divide" => {
            let v = p.value()?.string()?;
            let (x, y) = v.split_once('x').unwrap_or((&v, &v));
            let parse = |t: &str| {
                t.parse::<u32>()
                    .map_err(|_| Usage(format!("divide: {v} is not like 4 or 4x3")))
            };
            s.sample.size = Size::Relative {
                x: parse(x)?,
                y: parse(y)?,
            };
        }
        "sample" => {
            let v = p.value()?.string()?;
            s.sample.mode = SampleMode::from_name(&v).ok_or_else(|| {
                Usage(format!(
                    "unknown sampling mode {v}, see `xd-art-cli list samplers`"
                ))
            })?;
        }
        "offset" => {
            let v = p.value()?.string()?;
            let (x, y) = v
                .split_once(',')
                .ok_or_else(|| Usage(format!("offset: {v} is not like 0.25,-0.1")))?;
            s.sample.offset_x = x
                .trim()
                .parse()
                .map_err(|_| Usage(format!("offset: {x} is not a number")))?;
            s.sample.offset_y = y
                .trim()
                .parse()
                .map_err(|_| Usage(format!("offset: {y} is not a number")))?;
        }
        "edge-strength" => s.sample.edge_strength = num("edge-strength", p.value()?)?,
        "blur" => s.blur.radius = num("blur", p.value()?)?,
        "blur-compat" => s.blur.box_approx = true,
        "sharpen" => s.adjust.sharpen = num("sharpen", p.value()?)?,
        "brightness" => s.adjust.brightness = num("brightness", p.value()?)?,
        "contrast" => s.adjust.contrast = num("contrast", p.value()?)?,
        "saturation" => s.adjust.saturation = num("saturation", p.value()?)?,
        "hue" => s.adjust.hue = num("hue", p.value()?)?,
        "gamma" => s.adjust.gamma = num("gamma", p.value()?)?,
        "tint" => s.adjust.tint = hex(&p.value()?.string()?)?,
        "tint-strength" => s.adjust.tint_strength = num("tint-strength", p.value()?)?,
        "p" | "palette" => {
            let v = p.value()?.string()?;
            s.colors.palette = if xd_art_core::palette::builtin(&v).is_some() {
                PaletteSource::Builtin(v)
            } else {
                let pal =
                    xd_art_core::palette::Palette::load(std::path::Path::new(&v)).map_err(|e| {
                        Usage(format!(
                            "{v} is not a built-in palette and could not be loaded: {e}"
                        ))
                    })?;
                PaletteSource::Colors(pal.colors().to_vec())
            };
        }
        "lospec" => {
            let v = p.value()?.string()?;
            s.colors.palette = PaletteSource::Colors(crate::lospec(&v).map_err(Usage)?);
        }
        "extract" => {
            let count = num("extract", p.value()?)?;
            let (method, locked) = match &s.colors.palette {
                PaletteSource::Extract { method, locked, .. } => (*method, locked.clone()),
                _ => (Quantizer::KMeans, Vec::new()),
            };
            s.colors.palette = PaletteSource::Extract {
                count,
                method,
                locked,
            };
        }
        "method" => {
            let v = p.value()?.string()?;
            let m = match v.as_str() {
                "k-means" | "kmeans" => Quantizer::KMeans,
                "median-cut" => Quantizer::MedianCut,
                "balanced" => Quantizer::Balanced,
                _ => {
                    return Err(Usage(format!(
                        "unknown palette method {v}: k-means, median-cut or balanced"
                    )));
                }
            };
            match &mut s.colors.palette {
                PaletteSource::Extract { method, .. } => *method = m,
                other => {
                    *other = PaletteSource::Extract {
                        count: 16,
                        method: m,
                        locked: Vec::new(),
                    }
                }
            }
        }
        "lock" => {
            let c = hex(&p.value()?.string()?)?;
            match &mut s.colors.palette {
                PaletteSource::Extract { locked, .. } => locked.push(c),
                other => {
                    *other = PaletteSource::Extract {
                        count: 16,
                        method: Quantizer::KMeans,
                        locked: vec![c],
                    }
                }
            }
        }
        "metric" => {
            let v = p.value()?.string()?;
            s.colors.metric = Metric::from_name(&v).ok_or_else(|| {
                Usage(format!("unknown metric {v}, see `xd-art-cli list metrics`"))
            })?;
        }
        "d" | "dither" => {
            let v = p.value()?.string()?;
            s.dither.mode = DitherMode::from_name(&v).ok_or_else(|| {
                Usage(format!(
                    "unknown dither mode {v}, see `xd-art-cli list dithers`"
                ))
            })?;
        }
        "amount" => s.dither.amount = num("amount", p.value()?)?,
        "alpha-threshold" => s.dither.alpha_threshold = num("alpha-threshold", p.value()?)?,
        "target-colors" => s.dither.target_colors = num("target-colors", p.value()?)?,
        "mask" => o.mask = Some(p.value()?.into()),
        "outline" => s.post.outline = Some(num("outline", p.value()?)?),
        "inline" => s.post.inline = Some(num("inline", p.value()?)?),
        "cleanup" => s.post.cleanup = num("cleanup", p.value()?)?,
        "merge" => s.post.merge = num("merge", p.value()?)?,
        "s" | "upscale" => s.output.scale = num("upscale", p.value()?)?,
        "scanlines" => o.effects.scanlines = num("scanlines", p.value()?)?,
        "crt" => o.effects.crt = num("crt", p.value()?)?,
        "gif-palette" => {
            let v = p.value()?.string()?;
            o.gif_palette = match v.as_str() {
                "shared" => PaletteMode::Shared,
                "per-frame" => PaletteMode::PerFrame,
                _ => {
                    return Err(Usage(format!(
                        "gif-palette: {v} is not shared or per-frame"
                    )));
                }
            };
        }
        "handoff" => o.handoff = Some(p.value()?.into()),
        "suggest" => o.suggest = true,
        "json" => o.json = true,
        "q" | "quiet" => o.quiet = true,
        "f" | "force" => o.overwrite = true,
        "j" | "threads" => o.threads = Some(num("threads", p.value()?)?),
        _ => return Ok(false),
    }
    Ok(true)
}
