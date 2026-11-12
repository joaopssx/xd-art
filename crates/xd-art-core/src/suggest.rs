//! Picking a target size, and rendering several sizes at once.
//!
//! The suggestion is meant to be explained in one sentence. For each candidate width
//! the source is reduced to that size with a box filter, blown back up with nearest,
//! and compared with the source. The difference is the detail that size loses. The
//! suggestion is the smallest candidate that keeps at least three quarters of what the
//! largest candidate keeps, because pixel art wants to be as small as it can get away
//! with.

use crate::buffer::{Image, Work};
use crate::cancel::Cancel;
use crate::pipeline::{CachePolicy, Pipeline};
use crate::pool::Pool;
use crate::settings::{SampleMode, Settings, Size};
use crate::{Result, sampling};

/// Widths tried by [`suggest`], when the source is at least that wide.
pub const CANDIDATES: [u32; 9] = [16, 24, 32, 48, 64, 96, 128, 192, 256];

/// The comparison runs on the source reduced to at most this many pixels wide, which
/// keeps a suggestion for a 24 megapixel photo in the tens of milliseconds.
const PROBE_WIDTH: u32 = 512;

/// One candidate size and how much it loses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels, from the source aspect ratio.
    pub height: u32,
    /// Mean absolute difference after the round trip, in 8-bit steps per channel.
    pub error: f32,
}

/// The candidates and the one picked.
#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    /// Every size that was tried, small to large.
    pub candidates: Vec<Candidate>,
    /// Index of the suggested size in `candidates`.
    pub pick: usize,
}

impl Suggestion {
    /// The suggested size.
    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        let c = self.candidates[self.pick];
        (c.width, c.height)
    }

    /// One line on why this size, for the interface and the CLI.
    #[must_use]
    pub fn reason(&self) -> String {
        let c = self.candidates[self.pick];
        let first = self.candidates[0].error;
        let last = self.candidates[self.candidates.len() - 1].error;
        let kept = if first > last {
            (first - c.error) / (first - last) * 100.0
        } else {
            100.0
        };
        format!(
            "{}x{} keeps {kept:.0}% of the detail the largest size tried keeps (average error {:.1} per channel)",
            c.width, c.height, c.error
        )
    }
}

fn round_trip_error(probe: &Work, w: u32, h: u32, pool: &Pool, cancel: &Cancel) -> Result<f32> {
    let small = sampling::sample(probe, w, h, SampleMode::Box, (0.0, 0.0), pool, cancel)?;
    let mut total = 0u64;
    for y in 0..probe.height {
        let sy = (u64::from(y) * u64::from(h) / u64::from(probe.height)) as u32;
        for x in 0..probe.width {
            let sx = (u64::from(x) * u64::from(w) / u64::from(probe.width)) as u32;
            let (a, b) = (probe.at(x, y), small.at(sx, sy));
            for c in 0..3 {
                total += u64::from(a[c].abs_diff(b[c]) >> 7);
            }
        }
    }
    pool.recycle(small);
    Ok(total as f32 / (probe.px.len() * 3) as f32)
}

/// Suggests a target size for `img`.
///
/// # Errors
///
/// Fails when the run is cancelled.
pub fn suggest(img: &Image, cancel: &Cancel) -> Result<Suggestion> {
    let pool = Pool::new();
    let src = img.to_work();
    let (pw, ph) = if img.width() > PROBE_WIDTH {
        (
            PROBE_WIDTH,
            (u64::from(img.height()) * u64::from(PROBE_WIDTH) / u64::from(img.width())).max(1)
                as u32,
        )
    } else {
        (img.width(), img.height())
    };
    let probe = sampling::sample(&src, pw, ph, SampleMode::Box, (0.0, 0.0), &pool, cancel)?;
    let mut candidates = Vec::new();
    for &w in CANDIDATES.iter().filter(|&&w| w <= img.width().max(16)) {
        cancel.check()?;
        let h = Size::Absolute {
            width: w,
            height: 0,
            keep_aspect: true,
        }
        .resolve(img.width(), img.height())
        .1;
        let (cw, ch) = (w.min(pw), h.min(ph));
        let error = round_trip_error(&probe, cw.max(1), ch.max(1), &pool, cancel)?;
        candidates.push(Candidate {
            width: w,
            height: h,
            error,
        });
    }
    let first = candidates[0].error;
    let last = candidates[candidates.len() - 1].error;
    let goal = first - 0.75 * (first - last);
    let pick = candidates
        .iter()
        .position(|c| c.error <= goal)
        .unwrap_or(candidates.len() - 1);
    Ok(Suggestion { candidates, pick })
}

/// Converts `img` at several widths (height from the aspect ratio) with otherwise the
/// same settings, in parallel. Results are in the order of `widths`.
///
/// # Errors
///
/// Fails when a conversion fails or the run is cancelled.
pub fn previews(img: &Image, s: &Settings, widths: &[u32], cancel: &Cancel) -> Result<Vec<Image>> {
    let one = |p: &mut Pipeline, &w: &u32| -> Result<Image> {
        let mut st = s.clone();
        st.sample.size = Size::Absolute {
            width: w,
            height: 0,
            keep_aspect: true,
        };
        st.output.scale = 1;
        p.run(img, 1, &st, cancel)?;
        p.output().cloned().ok_or(crate::Error::Cancelled)
    };
    // Each pipeline keeps its blurred source between widths, so the blur runs once
    // per thread instead of once per width.
    let policy = CachePolicy {
        entries_per_stage: 0,
        ..CachePolicy::default()
    };
    #[cfg(feature = "parallel")]
    {
        use rayon::prelude::*;
        widths
            .par_iter()
            .map_init(|| Pipeline::new(policy), one)
            .collect()
    }
    #[cfg(not(feature = "parallel"))]
    {
        let mut p = Pipeline::new(policy);
        widths.iter().map(|w| one(&mut p, w)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A picture made of 8x8 blocks: anything at or above 1/8 of its size keeps all
    /// detail.
    fn blocks(w: u32, h: u32) -> Image {
        let mut img = Image::new(w, h, [0; 3]).unwrap();
        for y in 0..h {
            for x in 0..w {
                let v = (((x / 8) * 37 + (y / 8) * 91) % 256) as u8;
                img.set(x, y, [v, 255 - v, v / 2, 255]);
            }
        }
        img
    }

    #[test]
    fn blocky_image_suggests_its_block_size() {
        let img = blocks(256, 128);
        let s = suggest(&img, &Cancel::new()).unwrap();
        assert_eq!(s.size(), (32, 16), "{:?}", s.candidates);
        assert!(s.candidates[s.pick].error < 0.5);
        assert!(s.reason().starts_with("32x16 keeps"), "{}", s.reason());
        assert!(s.candidates.windows(2).all(|c| c[0].width < c[1].width));
    }

    #[test]
    fn small_sources_still_get_an_answer() {
        let img = blocks(10, 6);
        let s = suggest(&img, &Cancel::new()).unwrap();
        assert_eq!(s.candidates.len(), 1);
        assert_eq!(s.size().0, 16);
    }

    #[test]
    fn previews_come_in_order() {
        let img = blocks(128, 64);
        let out = previews(&img, &Settings::default(), &[32, 16, 64], &Cancel::new()).unwrap();
        let sizes: Vec<_> = out.iter().map(|i| (i.width(), i.height())).collect();
        assert_eq!(sizes, [(32, 16), (16, 8), (64, 32)]);
    }
}
