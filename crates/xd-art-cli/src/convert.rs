//! One conversion: read, convert, write.

use std::path::{Path, PathBuf};
use std::time::Instant;

use xd_art_core::buffer::Image;
use xd_art_core::cancel::Cancel;
use xd_art_core::pipeline::Pipeline;
use xd_art_core::settings::Mask;
use xd_art_core::{Error, anim, export, io};

use crate::args::Options;

/// What a conversion produced, for the human and the JSON report.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub input: PathBuf,
    pub output: PathBuf,
    pub width: u32,
    pub height: u32,
    pub colors: usize,
    pub frames: usize,
    pub millis: f64,
}

fn is_gif(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("gif"))
}

/// Loads a still image. A GIF gives its first frame.
pub fn load_still(path: &Path) -> Result<Image, Error> {
    if is_gif(path) {
        let a = anim::load_gif(path)?;
        return Ok(a
            .frames
            .into_iter()
            .next()
            .map(|f| f.image)
            .unwrap_or(Image::new(1, 1, [0; 3])?));
    }
    io::load(path)
}

/// Converts `input` into `output`.
pub fn run(input: &Path, output: &Path, o: &Options) -> Result<Outcome, Error> {
    let start = Instant::now();
    if output.exists() && !o.overwrite {
        return Err(Error::Invalid {
            name: "output",
            reason: format!("{} exists, use --force to replace it", output.display()),
        });
    }
    let mut settings = o.settings.clone();
    if let Some(m) = &o.mask {
        settings.mask = Some(Mask::from_image(&io::load(m)?)?);
    }
    if let Some(path) = &o.save_recipe {
        std::fs::write(path, xd_art_core::recipe::to_json(&settings)).map_err(|source| {
            Error::Io {
                path: path.clone(),
                source,
            }
        })?;
    }
    if is_gif(input) && is_gif(output) {
        let a = anim::load_gif(input)?;
        let conv = anim::convert(&a, &settings, o.gif_palette, &Cancel::new())?;
        let bytes = anim::encode_gif(&conv, settings.output.scale)?;
        std::fs::write(output, bytes).map_err(|source| Error::Io {
            path: output.into(),
            source,
        })?;
        let f = &conv.frames[0];
        return Ok(Outcome {
            input: input.into(),
            output: output.into(),
            width: f.width * settings.output.scale,
            height: f.height * settings.output.scale,
            colors: conv.palettes[0].len(),
            frames: conv.frames.len(),
            millis: start.elapsed().as_secs_f64() * 1000.0,
        });
    }
    let src = load_still(input)?;
    let mut p = Pipeline::default();
    p.run(&src, 1, &settings, &Cancel::new())?;
    let ix = p.indexed().ok_or(Error::Cancelled)?;
    let palette = p.palette().ok_or(Error::Cancelled)?.to_vec();
    let mut used = vec![false; palette.len()];
    for (i, &c) in ix.index.iter().enumerate() {
        if !ix.is_clear(i)
            && let Some(u) = used.get_mut(c as usize)
        {
            *u = true;
        }
    }
    let colors = used.iter().filter(|&&u| u).count();
    let mut img = p.output().ok_or(Error::Cancelled)?.clone();
    if o.effects.any() {
        img = o.effects.apply(&img, settings.output.scale)?;
    }
    if is_gif(output) {
        let conv = anim::Converted {
            frames: vec![ix.clone()],
            palettes: vec![palette.clone()],
            delays: vec![0],
        };
        let bytes = anim::encode_gif(&conv, settings.output.scale)?;
        std::fs::write(output, bytes).map_err(|source| Error::Io {
            path: output.into(),
            source,
        })?;
    } else {
        io::save(&img, output)?;
    }
    if let Some(dir) = &o.handoff {
        std::fs::create_dir_all(dir).map_err(|source| Error::Io {
            path: dir.clone(),
            source,
        })?;
        let stem = output.file_stem().and_then(|s| s.to_str()).unwrap_or("art");
        let pal = xd_art_core::palette::Palette::new(stem, palette)?;
        export::handoff(ix, &pal, &settings, dir, stem)?;
    }
    Ok(Outcome {
        input: input.into(),
        output: output.into(),
        width: img.width(),
        height: img.height(),
        colors,
        frames: 1,
        millis: start.elapsed().as_secs_f64() * 1000.0,
    })
}
