//! `batch`: the same settings over many files.

use std::path::{Path, PathBuf};

use crate::args::{Options, Usage};
use xd_art_text::{fill, tr};

const IMAGE_EXTS: [&str; 7] = ["png", "jpg", "jpeg", "bmp", "tga", "qoi", "gif"];

/// `*` and `?` matching on a file name, case insensitive like the file systems people
/// use this on most.
fn glob_match(pattern: &str, name: &str) -> bool {
    let (p, n): (Vec<char>, Vec<char>) = (
        pattern.to_lowercase().chars().collect(),
        name.to_lowercase().chars().collect(),
    );
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

fn is_image(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

fn walk(
    dir: &Path,
    recursive: bool,
    filter: &dyn Fn(&Path) -> bool,
    out: &mut Vec<PathBuf>,
) -> std::io::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            if recursive {
                walk(&p, true, filter, out)?;
            }
        } else if filter(&p) {
            out.push(p);
        }
    }
    Ok(())
}

/// Expands the inputs: files as they are, directories to the images inside, and
/// patterns with `*` or `?` in the last part to the matching files.
pub fn expand(inputs: &[String], recursive: bool) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    for i in inputs {
        let p = Path::new(i);
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.contains(['*', '?']) {
            let dir = p
                .parent()
                .filter(|d| !d.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            walk(
                dir,
                recursive,
                &|f: &Path| {
                    f.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| glob_match(name, n))
                },
                &mut out,
            )
            .map_err(|e| fill(tr("could not read {0}: {1}"), &[&dir.display(), &e]))?;
        } else if p.is_dir() {
            walk(p, recursive, &is_image, &mut out)
                .map_err(|e| fill(tr("could not read {0}: {1}"), &[&p.display(), &e]))?;
        } else {
            out.push(p.to_path_buf());
        }
    }
    out.dedup();
    Ok(out)
}

/// Where a converted file goes.
pub fn target(
    input: &Path,
    dir: &Path,
    ext: Option<&str>,
    suffix: &str,
    texture_names: Option<u32>,
) -> PathBuf {
    let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("out");
    let src_ext = input
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("png")
        .to_ascii_lowercase();
    if let Some(size) = texture_names {
        return dir.join(xd_art_core::export::texture_name(stem, size));
    }
    // JPEG is not an output format; such inputs come out as PNG unless told otherwise.
    let ext = ext
        .map(str::to_string)
        .unwrap_or(if src_ext == "jpg" || src_ext == "jpeg" {
            "png".into()
        } else {
            src_ext
        });
    dir.join(format!("{stem}{suffix}.{ext}"))
}

/// Runs `batch`. Returns whether every file converted.
pub fn run(mut p: lexopt::Parser) -> Result<bool, Usage> {
    use lexopt::prelude::*;
    let mut o = Options::default();
    let mut inputs = Vec::new();
    let mut out_dir = None;
    let mut ext = None;
    let mut suffix = String::new();
    let mut recursive = false;
    let mut texture_names = false;
    let mut next_to_input = false;
    while let Some(arg) = p.next()? {
        let key = match arg {
            Short(c) => c.to_string(),
            Long(l) => l.to_string(),
            Value(v) => {
                inputs.push(v.string()?);
                continue;
            }
        };
        if crate::args::option(&key, &mut p, &mut o)? {
            continue;
        }
        match key.as_str() {
            "o" | "output" => out_dir = Some(PathBuf::from(p.value()?)),
            "format" => {
                ext = Some(
                    p.value()?
                        .string()?
                        .trim_start_matches('.')
                        .to_ascii_lowercase(),
                )
            }
            "suffix" => suffix = p.value()?.string()?,
            "r" | "recursive" => recursive = true,
            "next-to-input" => next_to_input = true,
            "texture-names" => texture_names = true,
            other => return Err(Usage(fill(tr("unknown option {0}, try --help"), &[&other]))),
        }
    }
    if inputs.is_empty() {
        return Err(Usage(
            tr("batch needs at least one file, folder or pattern").into(),
        ));
    }
    let out_dir = match (out_dir, next_to_input) {
        (Some(_), true) => {
            return Err(Usage(
                tr("use either -o or --next-to-input, not both").into(),
            ));
        }
        (None, false) => {
            return Err(Usage(
                tr("batch needs -o <folder> or --next-to-input").into(),
            ));
        }
        (dir, _) => dir,
    };
    // Next to the input, a suffix keeps the result from replacing the source.
    if next_to_input && suffix.is_empty() {
        suffix = "-px".into();
    }
    let files = expand(&inputs, recursive).map_err(Usage)?;
    if let Some(dir) = &out_dir {
        std::fs::create_dir_all(dir)
            .map_err(|e| Usage(fill(tr("could not create {0}: {1}"), &[&dir.display(), &e])))?;
    }
    let size = match o.settings.sample.size {
        xd_art_core::settings::Size::Absolute { width, .. } => width,
        xd_art_core::settings::Size::Relative { .. } => 0,
    };
    let (mut done, mut failed) = (0usize, 0usize);
    let threads = o.threads;
    crate::with_threads(threads, || {
        for f in &files {
            let dir = out_dir
                .as_deref()
                .or_else(|| f.parent())
                .unwrap_or(Path::new("."));
            let dest = target(
                f,
                dir,
                ext.as_deref(),
                &suffix,
                texture_names.then_some(size),
            );
            match crate::convert::run(f, &dest, &o) {
                Ok(out) => {
                    done += 1;
                    if o.json {
                        println!("{}", crate::json_line(&out, None));
                    } else if !o.quiet {
                        crate::report(&out, false);
                    }
                }
                Err(e) => {
                    failed += 1;
                    if o.json {
                        let blank = crate::convert::Outcome {
                            input: f.clone(),
                            output: dest,
                            width: 0,
                            height: 0,
                            colors: 0,
                            frames: 0,
                            millis: 0.0,
                        };
                        println!("{}", crate::json_line(&blank, Some(&e.to_string())));
                    } else {
                        eprintln!("{e}");
                    }
                }
            }
        }
    });
    if o.json {
        println!("{{\"summary\":true,\"converted\":{done},\"failed\":{failed}}}");
    } else if !o.quiet {
        println!(
            "{}",
            fill(tr("{0} converted, {1} failed"), &[&done, &failed])
        );
    }
    Ok(failed == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globbing() {
        assert!(glob_match("*.png", "a.PNG"));
        assert!(glob_match("frame_??.png", "frame_07.png"));
        assert!(!glob_match("frame_??.png", "frame_7.png"));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("a*b*c", "aXXbYYc"));
        assert!(!glob_match("a*b*c", "aXXbYY"));
    }

    #[test]
    fn targets() {
        let d = Path::new("out");
        assert_eq!(
            target(Path::new("x/photo.jpg"), d, None, "", None),
            d.join("photo.png")
        );
        assert_eq!(
            target(Path::new("a.bmp"), d, None, "_px", None),
            d.join("a_px.bmp")
        );
        assert_eq!(
            target(Path::new("a.bmp"), d, Some("qoi"), "", None),
            d.join("a.qoi")
        );
        assert_eq!(
            target(Path::new("Oak Log.png"), d, None, "", Some(16)),
            d.join("oak_log_16.png")
        );
    }
}
