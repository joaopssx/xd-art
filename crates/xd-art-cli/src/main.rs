//! `xd-art-cli`: convert images to pixel art from the command line.
//!
//! Exit codes: 0 when everything worked, 1 when a conversion failed, 2 for a usage
//! error (unknown flag, bad value, missing file name). See `docs/cli.md`.

mod args;
mod batch;
mod convert;

use std::path::PathBuf;
use std::process::ExitCode;

use lexopt::prelude::*;
use xd_art_core::settings::{DitherMode, Metric, SampleMode};

use args::{Options, Usage};

const HELP: &str = "\
xd-art-cli: convert images to pixel art

usage:
  xd-art-cli convert <input> -o <output> [options]
  xd-art-cli batch <files, folders or patterns>... -o <folder> [options]
  xd-art-cli run <script.lua> [args]...     (also: --script <file>)
  xd-art-cli info <file>...
  xd-art-cli list <palettes|presets|samplers|metrics|dithers>
  xd-art-cli --version

size and sampling:
  -w, --width <px>            target width (default 64)
  -h, --height <px>           target height (default 64)
      --keep-aspect           height follows the width
      --divide <n|nxm>        divide the source size instead
      --sample <mode>         nearest, bilinear, bicubic, lanczos, box, cluster, edge
      --offset <x,y>          sample offset in output pixels, -1..1
      --edge-strength <0..1>  for --sample edge
prepare and adjust:
      --blur <radius>         blur before sampling
      --blur-compat           use the original program's box blur
      --sharpen <amount>      --brightness <-1..1>  --contrast <x>  --saturation <x>
      --hue <degrees>         --gamma <x>  --tint <rrggbb>  --tint-strength <0..1>
colors:
  -p, --palette <name|file>   built-in name or .gpl/.hex/.pal/.png file
      --extract <n>           build an n color palette from the image
      --method <name>         k-means, median-cut or balanced
      --lock <rrggbb>         keep this color when extracting (repeat)
      --lospec <name|link>    download a palette from lospec.com (if built with it)
      --metric <name>         rgb, weighted, redmean, cie76, cie94, ciede2000
dithering:
  -d, --dither <mode>         see `list dithers`
      --amount <0..1>         --alpha-threshold <0..255>  --target-colors <n>
      --mask <image>          grayscale map of dithering strength
post and output:
      --outline <index>       --inline <index>  --cleanup <0..2>  --merge <distance>
  -s, --upscale <n>           repeat every pixel n times
      --scanlines <0..1>      --crt <0..1>
      --gif-palette <mode>    shared or per-frame, for animated output
      --handoff <dir>         also write indexed png, .gpl, .hex and .json there
settings files:
      --preset <name>         start from a style preset
      --recipe <file>         start from a recipe (or an SLK_img2pixel preset)
      --save-recipe <file>    write the settings used
batch:
  -r, --recursive             look into subfolders
      --format <ext>          output format (default: same as input, png for jpeg)
      --suffix <text>         added to every output name
      --texture-names         lowercase names with the size, like oak_log_16.png
other:
      --suggest               print a suggested size and exit
      --json                  machine readable output
  -q, --quiet                 no output on success
  -f, --force                 replace existing output files
  -j, --threads <n>           worker threads (default: all cores)
";

fn lospec(name: &str) -> Result<Vec<[u8; 3]>, String> {
    #[cfg(feature = "online-palettes")]
    {
        let cache = std::env::var_os("XD_ART_CACHE")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os(if cfg!(windows) {
                    "LOCALAPPDATA"
                } else {
                    "HOME"
                })
                .map(|h| {
                    PathBuf::from(h).join(if cfg!(windows) {
                        "xd-art/lospec"
                    } else {
                        ".cache/xd-art/lospec"
                    })
                })
            });
        xd_art_core::palette::lospec::fetch(name, cache.as_deref())
            .map(|p| p.colors().to_vec())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(feature = "online-palettes"))]
    {
        let _ = name;
        Err("this build has no lospec support (feature online-palettes)".into())
    }
}

fn print_list(what: &str) -> Result<(), Usage> {
    match what {
        "palettes" => {
            for b in xd_art_core::palette::BUILTINS {
                let n = xd_art_core::palette::builtin(b.name).map_or(0, |p| p.len());
                println!("{:<12} {n:>3} colors  {}", b.name, b.source);
            }
        }
        "presets" => {
            for p in xd_art_core::preset::PRESETS {
                println!("{:<14} {}", p.name, p.about);
            }
        }
        "samplers" => SampleMode::ALL
            .iter()
            .for_each(|m| println!("{}", m.name())),
        "metrics" => Metric::ALL.iter().for_each(|m| println!("{}", m.name())),
        "dithers" => DitherMode::ALL
            .iter()
            .for_each(|m| println!("{}", m.name())),
        other => {
            return Err(Usage(format!(
                "cannot list {other}: palettes, presets, samplers, metrics or dithers"
            )));
        }
    }
    Ok(())
}

fn info(files: &[PathBuf]) -> bool {
    let mut ok = true;
    for f in files {
        let gif = f
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("gif"));
        let probed = if gif {
            xd_art_core::anim::load_gif(f).map(|a| (a.width, a.height, a.frames.len()))
        } else {
            xd_art_core::io::probe(f).map(|(w, h)| (w, h, 1))
        };
        match probed {
            Ok((w, h, 1)) => println!("{}: {w}x{h}", f.display()),
            Ok((w, h, n)) => println!("{}: {w}x{h}, {n} frames", f.display()),
            Err(e) => {
                eprintln!("{e}");
                ok = false;
            }
        }
    }
    ok
}

fn report(out: &convert::Outcome, json: bool) {
    if json {
        println!("{}", crate::json_line(out, None));
    } else {
        let frames = if out.frames > 1 {
            format!(", {} frames", out.frames)
        } else {
            String::new()
        };
        println!(
            "{} -> {} ({}x{}, {} colors{frames}, {:.0} ms)",
            out.input.display(),
            out.output.display(),
            out.width,
            out.height,
            out.colors,
            out.millis
        );
    }
}

/// One JSON object per file, on one line. The fields are documented in `docs/cli.md`.
fn json_line(out: &convert::Outcome, error: Option<&str>) -> String {
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    match error {
        Some(e) => format!(
            "{{\"input\":\"{}\",\"output\":\"{}\",\"ok\":false,\"error\":\"{}\"}}",
            esc(&out.input.display().to_string()),
            esc(&out.output.display().to_string()),
            esc(e)
        ),
        None => format!(
            "{{\"input\":\"{}\",\"output\":\"{}\",\"ok\":true,\"width\":{},\"height\":{},\"colors\":{},\"frames\":{},\"ms\":{:.1}}}",
            esc(&out.input.display().to_string()),
            esc(&out.output.display().to_string()),
            out.width,
            out.height,
            out.colors,
            out.frames,
            out.millis
        ),
    }
}

fn run_convert(mut p: lexopt::Parser) -> Result<bool, Usage> {
    let mut o = Options::default();
    let mut input = None;
    let mut output = None;
    while let Some(arg) = p.next()? {
        let key = match arg {
            Short(c) => c.to_string(),
            Long(l) => l.to_string(),
            Value(v) if input.is_none() => {
                input = Some(PathBuf::from(v));
                continue;
            }
            Value(v) => {
                return Err(Usage(format!(
                    "unexpected argument {}",
                    v.to_string_lossy()
                )));
            }
        };
        if args::option(&key, &mut p, &mut o)? {
            continue;
        }
        match key.as_str() {
            "o" | "output" => output = Some(PathBuf::from(p.value()?)),
            "help" => {
                print!("{HELP}");
                return Ok(true);
            }
            other => return Err(Usage(format!("unknown option {other}, try --help"))),
        }
    }
    let input = input.ok_or_else(|| Usage("convert needs an input file".into()))?;
    with_threads(o.threads, || {
        if o.suggest {
            return match convert::load_still(&input).and_then(|img| {
                xd_art_core::suggest::suggest(&img, &xd_art_core::cancel::Cancel::new())
            }) {
                Ok(s) => {
                    let (w, h) = s.size();
                    if o.json {
                        println!(
                            "{{\"input\":\"{}\",\"width\":{w},\"height\":{h}}}",
                            input.display().to_string().replace('\\', "\\\\")
                        );
                    } else {
                        println!("{}", s.reason());
                    }
                    Ok(true)
                }
                Err(e) => {
                    eprintln!("{e}");
                    Ok(false)
                }
            };
        }
        let output = output
            .clone()
            .ok_or_else(|| Usage("convert needs -o <output>".into()))?;
        Ok(match convert::run(&input, &output, &o) {
            Ok(out) => {
                if !o.quiet || o.json {
                    report(&out, o.json);
                }
                true
            }
            Err(e) => {
                if o.json {
                    let blank = convert::Outcome {
                        input: input.clone(),
                        output,
                        width: 0,
                        height: 0,
                        colors: 0,
                        frames: 0,
                        millis: 0.0,
                    };
                    println!("{}", json_line(&blank, Some(&e.to_string())));
                } else {
                    eprintln!("{e}");
                }
                false
            }
        })
    })
}

fn with_threads<R: Send>(threads: Option<usize>, f: impl FnOnce() -> R + Send) -> R {
    match threads {
        Some(n) if n > 0 => xd_art_core::par::with_threads(n, f),
        _ => f(),
    }
}

fn run_script(p: &mut lexopt::Parser) -> Result<bool, Usage> {
    let path = p.value()?.string()?;
    let args: Vec<String> = p
        .raw_args()?
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    #[cfg(feature = "lua")]
    {
        let file = xd_art_lua::resolve(&path);
        match xd_art_lua::run_file(&file, &args) {
            Ok(()) => Ok(true),
            Err(e) => {
                eprintln!("{e}");
                Ok(false)
            }
        }
    }
    #[cfg(not(feature = "lua"))]
    {
        let _ = args;
        Err(Usage(format!(
            "cannot run {path}: this build has no Lua (feature lua)"
        )))
    }
}

fn run() -> Result<bool, Usage> {
    let mut p = lexopt::Parser::from_env();
    let Some(first) = p.next()? else {
        print!("{HELP}");
        return Ok(true);
    };
    match first {
        Long("version") | Short('V') => {
            println!("xd-art-cli {}", env!("CARGO_PKG_VERSION"));
            Ok(true)
        }
        Long("help") => {
            print!("{HELP}");
            Ok(true)
        }
        Long("script") => run_script(&mut p),
        Value(cmd) => match cmd.string()?.as_str() {
            "run" => run_script(&mut p),
            "convert" => run_convert(p),
            "batch" => batch::run(p),
            "info" => {
                let files: Vec<PathBuf> = p.raw_args()?.map(PathBuf::from).collect();
                if files.is_empty() {
                    return Err(Usage("info needs at least one file".into()));
                }
                Ok(info(&files))
            }
            "list" => {
                let what = p.value()?.string()?;
                print_list(&what)?;
                Ok(true)
            }
            other => Err(Usage(format!("unknown command {other}, try --help"))),
        },
        other => Err(other.unexpected().into()),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(Usage(msg)) => {
            eprintln!("xd-art-cli: {msg}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn json_lines_escape_paths() {
        let out = convert::Outcome {
            input: Path::new("a\\b \"c\".png").into(),
            output: "o.png".into(),
            width: 2,
            height: 3,
            colors: 4,
            frames: 1,
            millis: 1.25,
        };
        let line = json_line(&out, None);
        assert_eq!(
            line,
            r#"{"input":"a\\b \"c\".png","output":"o.png","ok":true,"width":2,"height":3,"colors":4,"frames":1,"ms":1.2}"#
        );
        assert!(json_line(&out, Some("bad")).ends_with(r#""ok":false,"error":"bad"}"#));
    }
}
