//! Mapping the adjusted image to the palette, with or without dithering.
//!
//! Ordered dithering is the original's: it does not compare against the threshold, it
//! nudges every channel by `amount / 8 * (threshold - 0.5)` of the full range and then
//! takes the closest palette color. The threshold maps come from `build.rs`.

use crate::buffer::{Indexed, Px, Work, to8};
use crate::cancel::Cancel;
use crate::color::{Cache, Matcher};
use crate::par;
use crate::pool::Pool;
use crate::sampling::is_clear;
use crate::settings::{Dither, DitherMode, Metric};
use crate::tables;
use crate::{Error, Result};

/// The threshold map of an ordered mode, with its side length.
#[must_use]
pub fn threshold_map(mode: DitherMode) -> Option<(usize, &'static [f32])> {
    Some(match mode {
        DitherMode::Bayer2 => (2, &tables::BAYER2),
        DitherMode::Bayer3 => (3, &tables::BAYER3),
        DitherMode::Bayer4 => (4, &tables::BAYER4),
        DitherMode::Bayer5 => (5, &tables::BAYER5),
        DitherMode::Bayer8 => (8, &tables::BAYER8),
        DitherMode::Cluster4 => (4, &tables::CLUSTER4),
        DitherMode::Cluster8 => (8, &tables::CLUSTER8),
        _ => return None,
    })
}

#[inline]
fn rgb8(p: Px) -> [u8; 3] {
    [to8(p[0]), to8(p[1]), to8(p[2])]
}

/// Offset added to every channel by the ordered dither for one threshold value. The
/// original computes it in `f32` and truncates it to an integer.
#[inline]
fn nudge(amount: f32, t: f32) -> i64 {
    (32767.0 * (amount / 8.0) * (t - 0.5)) as i64
}

/// Maps `img` to `palette`. Transparent pixels (by the alpha threshold) get index 0 and
/// are marked clear.
///
/// # Errors
///
/// Fails when the palette is empty or has more than 256 colors, when the mode is not
/// implemented here, or when the run is cancelled.
pub fn map(
    img: &Work,
    palette: &[[u8; 3]],
    metric: Metric,
    d: &Dither,
    pool: &Pool,
    cancel: &Cancel,
) -> Result<Indexed> {
    if palette.is_empty() || palette.len() > 256 {
        return Err(Error::Invalid {
            name: "palette",
            reason: "needs 1 to 256 colors".into(),
        });
    }
    let m = Matcher::new(palette, metric);
    let mut out = pool.indexed(img.width, img.height);
    match d.mode {
        DitherMode::None => closest(img, &m, d.alpha_threshold, &mut out, cancel)?,
        mode => match threshold_map(mode) {
            Some((n, map)) => ordered(img, &m, *d, n, map, &mut out, cancel)?,
            None => {
                return Err(Error::Unsupported(format!("dither mode {}", mode.name())));
            }
        },
    }
    mark_clear(img, d.alpha_threshold, &mut out);
    Ok(out)
}

fn mark_clear(img: &Work, threshold: u8, out: &mut Indexed) {
    for (i, p) in img.px.iter().enumerate() {
        if is_clear(*p, threshold) {
            out.set_clear(i, true);
        }
    }
}

fn closest(
    img: &Work,
    m: &Matcher,
    threshold: u8,
    out: &mut Indexed,
    cancel: &Cancel,
) -> Result<()> {
    let w = img.width as usize;
    let cost = (m.colors().len() / 4).max(1);
    par::rows_with(
        &mut out.index,
        w,
        cost,
        cancel,
        Cache::default,
        |cache, y, row| {
            for (o, p) in row.iter_mut().zip(&img.px[y * w..(y + 1) * w]) {
                *o = if is_clear(*p, threshold) {
                    0
                } else {
                    cache.closest(m, rgb8(*p))
                };
            }
        },
    )
}

fn ordered(
    img: &Work,
    m: &Matcher,
    d: Dither,
    n: usize,
    map: &[f32],
    out: &mut Indexed,
    cancel: &Cancel,
) -> Result<()> {
    let w = img.width as usize;
    let offsets: Vec<i64> = map.iter().map(|&t| nudge(d.amount, t)).collect();
    let cost = (m.colors().len() / 4).max(1);
    par::rows_with(
        &mut out.index,
        w,
        cost,
        cancel,
        Cache::default,
        |cache, y, row| {
            let line = &offsets[(y % n) * n..(y % n + 1) * n];
            for (x, (o, p)) in row.iter_mut().zip(&img.px[y * w..(y + 1) * w]).enumerate() {
                if is_clear(*p, d.alpha_threshold) {
                    *o = 0;
                    continue;
                }
                let k = line[x % n];
                let q = [0, 1, 2].map(|c| (i64::from(p[c]) + k).clamp(0, 0x7fff) as u16);
                *o = cache.closest(m, [to8(q[0]), to8(q[1]), to8(q[2])]);
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::{MAX15, to15};

    #[test]
    fn tables_match_the_original() {
        let k = |v: u32, n: f32| v as f32 / n;
        assert_eq!(tables::BAYER2, [0, 2, 3, 1].map(|v| k(v, 4.0)));
        assert_eq!(
            tables::BAYER4,
            [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5].map(|v| k(v, 16.0))
        );
        assert_eq!(
            tables::BAYER8[..8],
            [0, 32, 8, 40, 2, 34, 10, 42].map(|v| k(v, 64.0))
        );
        assert_eq!(
            tables::BAYER8[56..],
            [63, 31, 55, 23, 61, 29, 53, 21].map(|v| k(v, 64.0))
        );
        assert_eq!(tables::BAYER3[4], 0.0);
        assert_eq!(tables::CLUSTER8[9], 0.0);
        for t in [&tables::BAYER8[..], &tables::CLUSTER8, &tables::BAYER5] {
            let mut v: Vec<f32> = t.to_vec();
            v.sort_by(f32::total_cmp);
            v.dedup();
            assert_eq!(v.len(), t.len(), "every level appears once");
        }
    }

    #[test]
    fn srgb_table_matches_the_formula() {
        for v in 0..=255u8 {
            let c = f32::from(v) / 255.0;
            let l = if c > 0.04045 {
                ((c + 0.055) / 1.055).powf(2.4)
            } else {
                c / 12.92
            };
            assert_eq!(tables::SRGB_TO_LINEAR[v as usize], l);
        }
    }

    #[test]
    fn closest_and_alpha_threshold() {
        let mut img = Work::new(2, 1, [to15(250), to15(250), to15(250), MAX15]).unwrap();
        img.px[1][3] = to15(10);
        let d = Dither::default();
        let out = map(
            &img,
            &[[0; 3], [255; 3]],
            Metric::Rgb,
            &d,
            &Pool::new(),
            &Cancel::new(),
        )
        .unwrap();
        assert_eq!(out.index, [1, 0]);
        assert!(!out.is_clear(0) && out.is_clear(1));
    }

    #[test]
    fn ordered_dither_mixes_two_colors() {
        let img = Work::new(8, 8, [to15(128), to15(128), to15(128), MAX15]).unwrap();
        let d = Dither {
            mode: DitherMode::Bayer4,
            amount: 1.0,
            ..Dither::default()
        };
        let out = map(
            &img,
            &[[0; 3], [255; 3]],
            Metric::Rgb,
            &d,
            &Pool::new(),
            &Cancel::new(),
        )
        .unwrap();
        let whites: usize = out.index.iter().map(|&i| usize::from(i == 1)).sum();
        assert!(whites > 0 && whites < 64, "{whites}");
    }

    #[test]
    fn bad_palettes_are_rejected() {
        let img = Work::new(1, 1, [0; 4]).unwrap();
        let d = Dither::default();
        assert!(map(&img, &[], Metric::Rgb, &d, &Pool::new(), &Cancel::new()).is_err());
    }
}
