//! Animated GIF: decoding into full frames, converting every frame, encoding again.
//!
//! Frames are converted in parallel, each thread with its own [`Pipeline`] so buffers
//! are reused from one frame to the next, and the results are put back in order.
//!
//! With a palette built from the image, the frames can share one palette (computed
//! once from all frames together, before any frame is mapped, so it does not flicker)
//! or get one each. A built-in or explicit palette is always shared.

use std::borrow::Cow;
use std::path::Path;

use crate::buffer::{Image, Indexed, Work};
use crate::cancel::Cancel;
use crate::pipeline::{CachePolicy, Pipeline};
use crate::pool::Pool;
use crate::settings::{PaletteSource, Settings};
use crate::{Error, Result, prep, quant, sampling};

/// One frame of an animation, already composed onto the full canvas.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// The pixels.
    pub image: Image,
    /// Time to show this frame, in hundredths of a second.
    pub delay: u16,
}

/// A decoded animation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Animation {
    /// Canvas width.
    pub width: u32,
    /// Canvas height.
    pub height: u32,
    /// Frames in order.
    pub frames: Vec<Frame>,
}

fn gif_err(e: &dyn std::fmt::Display) -> Error {
    Error::Decode {
        path: "<memory>".into(),
        reason: e.to_string(),
    }
}

/// Decodes a GIF, applying each frame's disposal so every [`Frame`] is a complete
/// picture.
///
/// # Errors
///
/// Fails on a broken or empty file.
pub fn decode_gif(bytes: &[u8]) -> Result<Animation> {
    let mut opts = gif::DecodeOptions::new();
    opts.set_color_output(gif::ColorOutput::RGBA);
    let mut dec = opts.read_info(bytes).map_err(|e| gif_err(&e))?;
    let (w, h) = (u32::from(dec.width()), u32::from(dec.height()));
    if w == 0 || h == 0 {
        return Err(gif_err(&"canvas has no pixels"));
    }
    let mut canvas = vec![0u8; w as usize * h as usize * 4];
    let mut frames = Vec::new();
    while let Some(f) = dec.read_next_frame().map_err(|e| gif_err(&e))? {
        let previous = (f.dispose == gif::DisposalMethod::Previous).then(|| canvas.clone());
        let (fx, fy, fw) = (
            usize::from(f.left),
            usize::from(f.top),
            usize::from(f.width),
        );
        for (i, px) in f.buffer.chunks_exact(4).enumerate() {
            let (x, y) = (fx + i % fw, fy + i / fw);
            if px[3] == 0 || x >= w as usize || y >= h as usize {
                continue;
            }
            let o = (y * w as usize + x) * 4;
            canvas[o..o + 4].copy_from_slice(px);
        }
        frames.push(Frame {
            image: Image::from_rgba(w, h, &canvas)?,
            delay: f.delay,
        });
        match f.dispose {
            gif::DisposalMethod::Background => {
                for y in fy..(fy + usize::from(f.height)).min(h as usize) {
                    for x in fx..(fx + fw).min(w as usize) {
                        let o = (y * w as usize + x) * 4;
                        canvas[o..o + 4].fill(0);
                    }
                }
            }
            gif::DisposalMethod::Previous => {
                if let Some(p) = previous {
                    canvas = p;
                }
            }
            _ => {}
        }
    }
    if frames.is_empty() {
        return Err(gif_err(&"no frames"));
    }
    Ok(Animation {
        width: w,
        height: h,
        frames,
    })
}

/// Loads a GIF file.
///
/// # Errors
///
/// Fails when the file cannot be read or decoded.
pub fn load_gif(path: &Path) -> Result<Animation> {
    let bytes = std::fs::read(path).map_err(|source| Error::Io {
        path: path.into(),
        source,
    })?;
    decode_gif(&bytes).map_err(|e| match e {
        Error::Decode { reason, .. } => Error::Decode {
            path: path.into(),
            reason,
        },
        other => other,
    })
}

/// Whether frames share a palette built from the image.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PaletteMode {
    /// One palette from all frames together.
    #[default]
    Shared,
    /// A palette per frame. Colors fit each frame better but can shift between frames.
    PerFrame,
}

/// Converted frames, ready to encode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Converted {
    /// Indexed frames, in order.
    pub frames: Vec<Indexed>,
    /// One palette per frame. With a shared palette every entry is the same.
    pub palettes: Vec<Vec<[u8; 3]>>,
    /// Delays in hundredths of a second.
    pub delays: Vec<u16>,
}

/// Builds the shared palette from the sampled and adjusted pixels of every frame.
fn shared_palette(anim: &Animation, s: &Settings, cancel: &Cancel) -> Result<Vec<[u8; 3]>> {
    let PaletteSource::Extract {
        count,
        method,
        locked,
    } = &s.colors.palette
    else {
        return Err(Error::Invalid {
            name: "palette",
            reason: "not extracted".into(),
        });
    };
    let pool = Pool::new();
    let mut all: Vec<crate::buffer::Px> = Vec::new();
    let mut width = 0;
    for f in &anim.frames {
        cancel.check()?;
        let mut w = f.image.to_work();
        prep::blur(&mut w, &s.blur, &pool, cancel)?;
        let mut small = sampling::run(&w, &s.sample, &pool, cancel)?;
        prep::adjust(&mut small, &s.adjust, &pool, cancel)?;
        width = small.width;
        all.extend_from_slice(&small.px);
    }
    let rows = (all.len() / width.max(1) as usize) as u32;
    let stacked = Work {
        width,
        height: rows,
        px: all,
    };
    Ok(quant::extract_locked(&stacked, *count, *method, locked)?
        .colors()
        .to_vec())
}

/// Converts every frame with `s`.
///
/// # Errors
///
/// Fails when a frame fails to convert or the run is cancelled.
pub fn convert(
    anim: &Animation,
    s: &Settings,
    mode: PaletteMode,
    cancel: &Cancel,
) -> Result<Converted> {
    let mut per = s.clone();
    if mode == PaletteMode::Shared && matches!(s.colors.palette, PaletteSource::Extract { .. }) {
        per.colors.palette = PaletteSource::Colors(shared_palette(anim, s, cancel)?);
    }
    // Output scale is applied when encoding, on indices.
    per.output.scale = 1;
    cancel.set_total(anim.frames.len() as u64);
    // Nothing to gain from older results: every frame is a new source.
    let policy = CachePolicy {
        entries_per_stage: 0,
        ..CachePolicy::default()
    };
    let one = |p: &mut Pipeline, (i, f): (usize, &Frame)| -> Result<(Indexed, Vec<[u8; 3]>)> {
        p.run(&f.image, i as u64 + 1, &per, cancel)?;
        cancel.advance(1);
        let ix = p.indexed().cloned().ok_or(Error::Cancelled)?;
        let pal = p
            .palette()
            .map(<[[u8; 3]]>::to_vec)
            .ok_or(Error::Cancelled)?;
        Ok((ix, pal))
    };
    #[cfg(feature = "parallel")]
    let results: Vec<Result<(Indexed, Vec<[u8; 3]>)>> = {
        use rayon::prelude::*;
        anim.frames
            .par_iter()
            .enumerate()
            .map_init(|| Pipeline::new(policy), one)
            .collect()
    };
    #[cfg(not(feature = "parallel"))]
    let results: Vec<Result<(Indexed, Vec<[u8; 3]>)>> = {
        let mut p = Pipeline::new(policy);
        anim.frames
            .iter()
            .enumerate()
            .map(|f| one(&mut p, f))
            .collect()
    };
    let mut out = Converted {
        frames: Vec::new(),
        palettes: Vec::new(),
        delays: Vec::new(),
    };
    for (r, f) in results.into_iter().zip(&anim.frames) {
        let (ix, pal) = r?;
        out.frames.push(ix);
        out.palettes.push(pal);
        out.delays.push(f.delay);
    }
    Ok(out)
}

fn scaled_indices(ix: &Indexed, transparent: Option<u8>, k: u32) -> Vec<u8> {
    let (w, k) = (ix.width as usize, k as usize);
    let mut out = Vec::with_capacity(ix.index.len() * k * k);
    for y in 0..ix.height as usize {
        let start = out.len();
        for x in 0..w {
            let i = y * w + x;
            let v = match transparent {
                Some(t) if ix.is_clear(i) => t,
                _ => ix.index[i],
            };
            out.extend(std::iter::repeat_n(v, k));
        }
        let line = out[start..].to_vec();
        for _ in 1..k {
            out.extend_from_slice(&line);
        }
    }
    out
}

/// Encodes converted frames as a looping GIF, every pixel repeated `scale` times.
///
/// Transparency needs a free palette slot: when a palette has fewer than 256 colors one
/// more entry is added for it; with a full palette, transparent pixels are written as
/// index 0.
///
/// # Errors
///
/// Fails when there are no frames, the size does not fit a GIF, or the encoder fails.
pub fn encode_gif(c: &Converted, scale: u32) -> Result<Vec<u8>> {
    let first = c.frames.first().ok_or_else(|| Error::Invalid {
        name: "animation",
        reason: "no frames".into(),
    })?;
    let (w, h) = (first.width * scale.max(1), first.height * scale.max(1));
    let (Ok(gw), Ok(gh)) = (u16::try_from(w), u16::try_from(h)) else {
        return Err(Error::Unsupported(format!(
            "gif of {w}x{h}, the limit is 65535"
        )));
    };
    let enc_err = |e: gif::EncodingError| Error::Encode {
        path: "<memory>".into(),
        reason: e.to_string(),
    };
    let shared = c.palettes.windows(2).all(|p| p[0] == p[1]);
    let any_clear = c.frames.iter().any(|f| f.clear.iter().any(|&x| x));
    let with_slot = |p: &[[u8; 3]]| {
        let mut flat: Vec<u8> = p.iter().flatten().copied().collect();
        let slot = (any_clear && p.len() < 256).then(|| {
            flat.extend([0, 0, 0]);
            p.len() as u8
        });
        (flat, slot)
    };
    let mut out = Vec::new();
    {
        let (global, global_slot) = if shared {
            with_slot(&c.palettes[0])
        } else {
            (Vec::new(), None)
        };
        let mut enc = gif::Encoder::new(&mut out, gw, gh, &global).map_err(enc_err)?;
        enc.set_repeat(gif::Repeat::Infinite).map_err(enc_err)?;
        for (i, ix) in c.frames.iter().enumerate() {
            let (local, slot) = if shared {
                (Vec::new(), global_slot)
            } else {
                with_slot(&c.palettes[i])
            };
            let frame = gif::Frame {
                width: gw,
                height: gh,
                delay: c.delays[i],
                transparent: slot,
                dispose: gif::DisposalMethod::Background,
                palette: (!shared).then_some(local),
                buffer: Cow::Owned(scaled_indices(
                    ix,
                    slot.or(Some(0)).filter(|_| any_clear),
                    scale.max(1),
                )),
                ..gif::Frame::default()
            };
            enc.write_frame(&frame).map_err(enc_err)?;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{Quantizer, Size};

    fn moving_square(frames: u32) -> Animation {
        let mut out = Vec::new();
        for f in 0..frames {
            let mut img = Image::new(24, 16, [20, 30, 160]).unwrap();
            for y in 4..12 {
                for x in 0..8 {
                    img.set(x + f * 2, y, [240, 200, 20, 255]);
                }
            }
            img.set(23, 15, [0, 0, 0, 0]);
            out.push(Frame {
                image: img,
                delay: 5 + f as u16,
            });
        }
        Animation {
            width: 24,
            height: 16,
            frames: out,
        }
    }

    fn settings() -> Settings {
        let mut s = Settings::default();
        s.sample.size = Size::Absolute {
            width: 12,
            height: 8,
            keep_aspect: false,
        };
        s
    }

    #[test]
    fn round_trip_keeps_frames_and_delays() {
        let anim = moving_square(4);
        let conv = convert(&anim, &settings(), PaletteMode::Shared, &Cancel::new()).unwrap();
        assert_eq!(conv.frames.len(), 4);
        let bytes = encode_gif(&conv, 2).unwrap();
        let back = decode_gif(&bytes).unwrap();
        assert_eq!((back.width, back.height), (24, 16));
        assert_eq!(back.frames.len(), 4);
        assert_eq!(
            back.frames.iter().map(|f| f.delay).collect::<Vec<_>>(),
            [5, 6, 7, 8]
        );
        let expect = conv.frames[2]
            .to_image(&conv.palettes[2])
            .unwrap()
            .upscale(2)
            .unwrap();
        assert_eq!(back.frames[2].image, expect);
    }

    #[test]
    fn shared_palette_is_the_same_for_every_frame() {
        let anim = moving_square(5);
        let mut s = settings();
        s.colors.palette = PaletteSource::Extract {
            count: 4,
            method: Quantizer::KMeans,
            locked: vec![],
        };
        let shared = convert(&anim, &s, PaletteMode::Shared, &Cancel::new()).unwrap();
        assert!(shared.palettes.windows(2).all(|p| p[0] == p[1]));
        let per = convert(&anim, &s, PaletteMode::PerFrame, &Cancel::new()).unwrap();
        assert_eq!(per.palettes.len(), 5);
        let bytes = encode_gif(&per, 1).unwrap();
        assert_eq!(decode_gif(&bytes).unwrap().frames.len(), 5);
    }

    #[test]
    fn order_and_result_do_not_depend_on_threads() {
        let anim = moving_square(9);
        let run = |t| {
            crate::par::with_threads(t, || {
                convert(&anim, &settings(), PaletteMode::Shared, &Cancel::new()).unwrap()
            })
        };
        assert_eq!(run(1), run(6));
    }

    #[test]
    fn progress_and_cancel() {
        let anim = moving_square(3);
        let c = Cancel::new();
        convert(&anim, &settings(), PaletteMode::Shared, &c).unwrap();
        assert_eq!(c.progress(), (3, 3));
        let stop = Cancel::new();
        stop.cancel();
        assert!(convert(&anim, &settings(), PaletteMode::Shared, &stop).is_err());
    }

    #[test]
    fn broken_input() {
        assert!(decode_gif(b"GIF89a").is_err());
        assert!(
            encode_gif(
                &Converted {
                    frames: vec![],
                    palettes: vec![],
                    delays: vec![]
                },
                1
            )
            .is_err()
        );
    }
}
