//! Mapping the adjusted image to the palette, with or without dithering.
//!
//! Ordered dithering is the original's: it does not compare against the threshold, it
//! nudges every channel by `amount / 8 * (threshold - 0.5)` of the full range and then
//! takes the closest palette color. The threshold maps come from `build.rs`.
//!
//! Error diffusion follows the original's arithmetic too: errors are measured in 8-bit
//! steps, spread in `f32`, and added back to the 15-bit working image with truncation.
//! Transparent pixels neither receive nor pass on error.
//!
//! Ordered modes, picoCAD and plain closest color run row by row in parallel. Error
//! diffusion is sequential by nature; it runs as a wavefront over threads, with a lag
//! between rows that makes the result identical to the one thread loop.

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
        DitherMode::Picocad => picocad(img, &m, d.alpha_threshold, &mut out, cancel)?,
        DitherMode::MedianCut => median_assign(img, &m, *d, &mut out),
        mode if mode.is_diffusion() => {
            let mut work = pool.copy(img);
            let r = diffuse(&mut work, &m, mode, d.alpha_threshold, &mut out, cancel);
            pool.recycle(work);
            r?;
        }
        mode => match threshold_map(mode) {
            Some((n, map)) => ordered(img, &m, *d, n, map, &mut out, cancel)?,
            None => return Err(Error::Unsupported(format!("dither mode {}", mode.name()))),
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

/// One target of an error diffusion kernel: offset and weight as numerator and
/// denominator.
#[derive(Clone, Copy, Debug)]
pub struct Tap {
    /// Horizontal offset.
    pub dx: i8,
    /// Vertical offset, never negative.
    pub dy: i8,
    /// Weight numerator.
    pub num: u8,
    /// Weight denominator.
    pub den: u8,
}

const fn tap(dx: i8, dy: i8, num: u8, den: u8) -> Tap {
    Tap { dx, dy, num, den }
}

/// Floyd-Steinberg.
pub const FLOYD: [Tap; 4] = [
    tap(1, 0, 7, 16),
    tap(-1, 1, 3, 16),
    tap(0, 1, 5, 16),
    tap(1, 1, 1, 16),
];
/// Stucki.
pub const STUCKI: [Tap; 12] = [
    tap(1, 0, 8, 42),
    tap(2, 0, 4, 42),
    tap(-2, 1, 2, 42),
    tap(-1, 1, 4, 42),
    tap(0, 1, 8, 42),
    tap(1, 1, 4, 42),
    tap(2, 1, 2, 42),
    tap(-2, 2, 1, 42),
    tap(-1, 2, 2, 42),
    tap(0, 2, 4, 42),
    tap(1, 2, 2, 42),
    tap(2, 2, 1, 42),
];
/// Burkes.
pub const BURKES: [Tap; 7] = [
    tap(1, 0, 8, 32),
    tap(2, 0, 4, 32),
    tap(-2, 1, 2, 32),
    tap(-1, 1, 4, 32),
    tap(0, 1, 8, 32),
    tap(1, 1, 4, 32),
    tap(2, 1, 2, 32),
];
/// Sierra (three rows).
pub const SIERRA: [Tap; 10] = [
    tap(1, 0, 5, 32),
    tap(2, 0, 3, 32),
    tap(-2, 1, 2, 32),
    tap(-1, 1, 4, 32),
    tap(0, 1, 5, 32),
    tap(1, 1, 4, 32),
    tap(2, 1, 2, 32),
    tap(-1, 2, 2, 32),
    tap(0, 2, 3, 32),
    tap(1, 2, 2, 32),
];
/// Two-row Sierra.
pub const SIERRA_TWO_ROW: [Tap; 7] = [
    tap(1, 0, 4, 16),
    tap(2, 0, 3, 16),
    tap(-2, 1, 1, 16),
    tap(-1, 1, 2, 16),
    tap(0, 1, 3, 16),
    tap(1, 1, 2, 16),
    tap(2, 1, 1, 16),
];
/// Sierra lite.
pub const SIERRA_LITE: [Tap; 3] = [tap(1, 0, 2, 4), tap(-1, 1, 1, 4), tap(0, 1, 1, 4)];

/// The kernel of an error diffusion mode.
#[must_use]
pub fn kernel(mode: DitherMode) -> Option<&'static [Tap]> {
    Some(match mode {
        DitherMode::Floyd | DitherMode::FloydMono => &FLOYD,
        DitherMode::Stucki => &STUCKI,
        DitherMode::Burkes => &BURKES,
        DitherMode::Sierra => &SIERRA,
        DitherMode::SierraTwoRow => &SIERRA_TWO_ROW,
        DitherMode::SierraLite => &SIERRA_LITE,
        _ => return None,
    })
}

/// Error per channel to add at one tap. The original computes Floyd-Steinberg as
/// `(e * n) / 16` and the other kernels as `e * w` with `w` a precomputed `n / d`, and
/// those round differently in `f32`, so both forms are kept.
#[inline]
fn share(mode: DitherMode, e: f32, t: Tap) -> f32 {
    if matches!(mode, DitherMode::Floyd | DitherMode::FloydMono) {
        (e * f32::from(t.num)) / f32::from(t.den)
    } else {
        e * (f32::from(t.num) / f32::from(t.den))
    }
}

/// Adds `e` (in 8-bit units) to a pixel of the working image, the original's way.
#[inline]
fn add_error(p: &mut Px, e: [f32; 3]) {
    for c in 0..3 {
        p[c] = ((f32::from(p[c]) + e[c] * 128.0) as i64).clamp(0, 0x7fff) as u16;
    }
}

/// Error left after mapping one pixel to palette color `pc`.
#[inline]
fn pixel_error(mode: DitherMode, p: Px, pc: [u8; 3]) -> [f32; 3] {
    let c = rgb8(p);
    let e = [0, 1, 2].map(|i| f32::from(c[i]) - f32::from(pc[i]));
    if mode == DitherMode::FloydMono {
        let mut m = e[0];
        m += e[1];
        m += e[2];
        [m / 3.0; 3]
    } else {
        e
    }
}

/// Error diffusion over the whole image, one pixel after the other in reading order.
/// `work` is modified as the error spreads.
fn diffuse_serial(
    work: &mut Work,
    m: &Matcher,
    mode: DitherMode,
    threshold: u8,
    out: &mut Indexed,
    cancel: &Cancel,
) -> Result<()> {
    let taps = kernel(mode).unwrap_or(&FLOYD);
    let (w, h) = (work.width as i32, work.height as i32);
    let mut cache = Cache::default();
    for y in 0..h {
        cancel.check()?;
        for x in 0..w {
            let i = (y * w + x) as usize;
            let p = work.px[i];
            if is_clear(p, threshold) {
                out.index[i] = 0;
                continue;
            }
            let c = cache.closest(m, rgb8(p));
            out.index[i] = c;
            let e = pixel_error(mode, p, m.colors()[c as usize]);
            for t in taps {
                let (tx, ty) = (x + i32::from(t.dx), y + i32::from(t.dy));
                if tx >= 0 && tx < w && ty < h {
                    let share = e.map(|v| share(mode, v, *t));
                    add_error(&mut work.px[(ty * w + tx) as usize], share);
                }
            }
        }
    }
    Ok(())
}

/// Below this many pixels the wavefront is not worth starting threads for.
#[cfg(feature = "parallel")]
const WAVE_FROM: usize = 64 * 1024;

#[cfg(feature = "parallel")]
#[inline]
fn pack(p: Px) -> u64 {
    u64::from(p[0]) | u64::from(p[1]) << 16 | u64::from(p[2]) << 32 | u64::from(p[3]) << 48
}

#[cfg(feature = "parallel")]
#[inline]
fn unpack(v: u64) -> Px {
    [
        v as u16,
        (v >> 16) as u16,
        (v >> 32) as u16,
        (v >> 48) as u16,
    ]
}

/// Error diffusion on several threads, with exactly the serial result.
///
/// Rows are dealt round robin to the threads, and row `y` only handles pixel `x` once
/// row `y - 1` has finished pixel `x + lag`, where `lag` is twice the kernel's reach
/// (4 for every kernel here). With that lag:
///
/// - every contribution a pixel will ever get from the rows above has arrived before
///   the pixel is read;
/// - contributions arrive in the same order as in the serial loop (all of row `y - 2`
///   before any of row `y - 1`), and since each one is rounded on its own, the order is
///   what makes the result identical, not just close;
/// - no two threads ever touch the same pixel at the same time.
///
/// Pixels are `AtomicU64` with relaxed access; the per row progress counters carry the
/// ordering (release after a pixel is done, acquire before reading the row below it).
#[cfg(feature = "parallel")]
fn diffuse_wave(
    work: &Work,
    m: &Matcher,
    mode: DitherMode,
    threshold: u8,
    out: &mut Indexed,
    cancel: &Cancel,
    threads: usize,
) -> Result<()> {
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

    let taps = kernel(mode).unwrap_or(&FLOYD);
    let reach = taps.iter().map(|t| t.dx.unsigned_abs()).max().unwrap_or(1) as usize;
    let lag = 2 * reach;
    let (w, h) = (work.width as usize, work.height as usize);
    let px: Vec<AtomicU64> = work.px.iter().map(|&p| AtomicU64::new(pack(p))).collect();
    let done: Vec<AtomicUsize> = (0..h).map(|_| AtomicUsize::new(0)).collect();
    let stop = AtomicBool::new(false);
    let rows: Vec<Vec<(usize, Vec<u8>)>> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                let (px, done, stop) = (&px, &done, &stop);
                s.spawn(move || {
                    let mut cache = Cache::default();
                    let mut mine = Vec::new();
                    for y in (t..h).step_by(threads) {
                        let mut line = vec![0u8; w];
                        for x in 0..w {
                            if y > 0 {
                                let need = (x + lag + 1).min(w);
                                let mut spins = 0u32;
                                while done[y - 1].load(Ordering::Acquire) < need {
                                    if stop.load(Ordering::Relaxed) {
                                        return mine;
                                    }
                                    spins += 1;
                                    if spins > 64 {
                                        std::thread::yield_now();
                                    } else {
                                        std::hint::spin_loop();
                                    }
                                }
                            }
                            let p = unpack(px[y * w + x].load(Ordering::Relaxed));
                            if !is_clear(p, threshold) {
                                let c = cache.closest(m, rgb8(p));
                                line[x] = c;
                                let e = pixel_error(mode, p, m.colors()[c as usize]);
                                for tp in taps {
                                    let tx = x as isize + isize::from(tp.dx);
                                    let ty = y + tp.dy as usize;
                                    if tx >= 0 && (tx as usize) < w && ty < h {
                                        let slot = &px[ty * w + tx as usize];
                                        let mut q = unpack(slot.load(Ordering::Relaxed));
                                        add_error(&mut q, e.map(|v| share(mode, v, *tp)));
                                        slot.store(pack(q), Ordering::Relaxed);
                                    }
                                }
                            }
                            done[y].store(x + 1, Ordering::Release);
                        }
                        mine.push((y, line));
                        if cancel.is_cancelled() {
                            stop.store(true, Ordering::Relaxed);
                            return mine;
                        }
                    }
                    mine
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|j| j.join().unwrap_or_default())
            .collect()
    });
    cancel.check()?;
    for (y, line) in rows.into_iter().flatten() {
        out.index[y * w..(y + 1) * w].copy_from_slice(&line);
    }
    Ok(())
}

/// Picks the serial loop or the wavefront.
fn diffuse(
    work: &mut Work,
    m: &Matcher,
    mode: DitherMode,
    threshold: u8,
    out: &mut Indexed,
    cancel: &Cancel,
) -> Result<()> {
    #[cfg(feature = "parallel")]
    {
        let threads = rayon::current_num_threads().min(work.height as usize);
        if threads > 1 && work.px.len() >= WAVE_FROM {
            return diffuse_wave(work, m, mode, threshold, out, cancel, threads);
        }
    }
    diffuse_serial(work, m, mode, threshold, out, cancel)
}

/// The picoCAD look: the two closest colors, and a checkerboard between them when the
/// pixel sits in the middle third of the line joining them.
fn picocad(
    img: &Work,
    m: &Matcher,
    threshold: u8,
    out: &mut Indexed,
    cancel: &Cancel,
) -> Result<()> {
    let w = img.width as usize;
    par::rows(
        &mut out.index,
        w,
        m.colors().len().max(1),
        cancel,
        |y, row| {
            for (x, (o, p)) in row.iter_mut().zip(&img.px[y * w..(y + 1) * w]).enumerate() {
                if is_clear(*p, threshold) {
                    *o = 0;
                    continue;
                }
                let (i0, i1, t) = m.closest_two(rgb8(*p));
                let checker_odd = (x ^ y) & 1 == 1;
                *o = if t >= 2.0 / 3.0 || (t > 1.0 / 3.0 && checker_odd) {
                    i1
                } else {
                    i0
                };
            }
        },
    )
}

/// Minimum cost assignment of rows to distinct columns (`rows <= cols`), with the
/// Hungarian method using potentials, in `O(rows^2 * cols)`. Returns the column of
/// every row.
fn assign(cost: &[f64], rows: usize, cols: usize) -> Vec<usize> {
    // 1-based arrays as in the usual formulation; column 0 is the virtual start.
    let inf = f64::INFINITY;
    let mut u = vec![0.0; rows + 1];
    let mut v = vec![0.0; cols + 1];
    let mut owner = vec![0usize; cols + 1];
    let mut way = vec![0usize; cols + 1];
    for r in 1..=rows {
        owner[0] = r;
        let mut j0 = 0;
        let mut minv = vec![inf; cols + 1];
        let mut used = vec![false; cols + 1];
        loop {
            used[j0] = true;
            let i0 = owner[j0];
            let mut delta = inf;
            let mut j1 = 0;
            for j in 1..=cols {
                if !used[j] {
                    let cur = cost[(i0 - 1) * cols + (j - 1)] - u[i0] - v[j];
                    if cur < minv[j] {
                        minv[j] = cur;
                        way[j] = j0;
                    }
                    if minv[j] < delta {
                        delta = minv[j];
                        j1 = j;
                    }
                }
            }
            for j in 0..=cols {
                if used[j] {
                    u[owner[j]] += delta;
                    v[j] -= delta;
                } else {
                    minv[j] -= delta;
                }
            }
            j0 = j1;
            if owner[j0] == 0 {
                break;
            }
        }
        loop {
            let j1 = way[j0];
            owner[j0] = owner[j1];
            j0 = j1;
            if j0 == 0 {
                break;
            }
        }
    }
    let mut out = vec![0; rows];
    for j in 1..=cols {
        if owner[j] != 0 {
            out[owner[j] - 1] = j - 1;
        }
    }
    out
}

/// The original's median cut assignment: split the opaque pixels into as many boxes as
/// target colors (cutting the widest channel at the middle of its range), then give
/// every box a different palette color so the total distance is as small as possible.
/// Large flat areas end up with one color each, which reads more like hand made pixel
/// art than per pixel matching.
fn median_assign(img: &Work, m: &Matcher, d: Dither, out: &mut Indexed) {
    let pal = m.colors();
    let target = usize::from(d.target_colors).clamp(1, pal.len());
    let mut items: Vec<([u8; 3], u32)> = img
        .px
        .iter()
        .enumerate()
        .filter(|(_, p)| to8(p[3]) >= d.alpha_threshold)
        .map(|(i, p)| (rgb8(*p), i as u32))
        .collect();
    if items.is_empty() {
        return;
    }
    let range = |s: &[([u8; 3], u32)]| {
        let mut lo = [255u8; 3];
        let mut hi = [0u8; 3];
        for (c, _) in s {
            for k in 0..3 {
                lo[k] = lo[k].min(c[k]);
                hi[k] = hi[k].max(c[k]);
            }
        }
        [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]]
    };
    let mut boxes: Vec<(usize, usize, [u8; 3])> = vec![(0, items.len(), range(&items))];
    while boxes.len() < target {
        let mut best = 0;
        let mut max = 0;
        for (i, b) in boxes.iter().enumerate() {
            let r = b.2[0].max(b.2[1]).max(b.2[2]);
            if r > max {
                max = r;
                best = i;
            }
        }
        if max == 0 {
            break;
        }
        let (start, end, r) = boxes[best];
        let ch = if max == r[0] {
            0
        } else if max == r[1] {
            1
        } else {
            2
        };
        // The original uses qsort, which is not stable; a stable sort gives one fixed
        // answer here.
        items[start..end].sort_by_key(|(c, _)| c[ch]);
        let cut = u32::midpoint(
            u32::from(items[start].0[ch]),
            u32::from(items[end - 1].0[ch]),
        );
        let len = items[start..end]
            .iter()
            .position(|(c, _)| u32::from(c[ch]) > cut)
            .unwrap_or(end - start);
        boxes[best] = (start, start + len, range(&items[start..start + len]));
        boxes.push((start + len, end, range(&items[start + len..end])));
    }
    let coords: Vec<_> = pal
        .iter()
        .map(|&c| crate::color::coords(m.metric(), c))
        .collect();
    let mut cost = vec![0f64; boxes.len() * pal.len()];
    for (b, &(start, end, _)) in boxes.iter().enumerate() {
        for (c, _) in &items[start..end] {
            let q = crate::color::coords(m.metric(), *c);
            for (p, pc) in coords.iter().enumerate() {
                // Palette first, pixel second: the order the original passes them in,
                // which matters for the asymmetric metrics.
                cost[b * pal.len() + p] += f64::from(crate::color::distance(m.metric(), *pc, q));
            }
        }
    }
    let pick = assign(&cost, boxes.len(), pal.len());
    for (b, &(start, end, _)) in boxes.iter().enumerate() {
        for (_, i) in &items[start..end] {
            out.index[*i as usize] = pick[b] as u8;
        }
    }
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
    fn kernels_sum_to_one() {
        for mode in DitherMode::ALL.into_iter().filter(|m| m.is_diffusion()) {
            let k = kernel(mode).unwrap();
            let sum: u32 = k.iter().map(|t| u32::from(t.num)).sum();
            assert_eq!(sum, u32::from(k[0].den), "{mode:?}");
            assert!(
                k.iter().all(|t| t.dy > 0 || t.dx > 0),
                "{mode:?} only pushes forward"
            );
        }
    }

    #[test]
    fn assignment_is_optimal() {
        // 3 rows, 4 columns; brute force over every injective choice.
        let cost = [4.0, 1.0, 3.0, 9.0, 2.0, 0.0, 5.0, 3.0, 3.0, 2.0, 2.0, 8.0];
        let got = assign(&cost, 3, 4);
        let total = |a: &[usize]| {
            a.iter()
                .enumerate()
                .map(|(r, &c)| cost[r * 4 + c])
                .sum::<f64>()
        };
        let mut best = f64::MAX;
        for a in 0..4 {
            for b in 0..4 {
                for c in 0..4 {
                    if a != b && b != c && a != c {
                        best = best.min(total(&[a, b, c]));
                    }
                }
            }
        }
        assert_eq!(total(&got), best);
        let mut cols = got.clone();
        cols.sort_unstable();
        cols.dedup();
        assert_eq!(cols.len(), 3);
    }

    #[test]
    fn median_cut_uses_distinct_colors_per_box() {
        let mut img = Work::new(4, 1, [0, 0, 0, MAX15]).unwrap();
        img.px[2] = [to15(250), to15(250), to15(250), MAX15];
        img.px[3] = [to15(240), to15(240), to15(240), MAX15];
        let d = Dither {
            mode: DitherMode::MedianCut,
            target_colors: 2,
            ..Dither::default()
        };
        let pal = [[255; 3], [0; 3], [128; 3]];
        let out = map(&img, &pal, Metric::Rgb, &d, &Pool::new(), &Cancel::new()).unwrap();
        assert_eq!(out.index, [1, 1, 0, 0]);
    }

    #[test]
    fn picocad_checkers_the_middle() {
        let img = Work::new(4, 4, [to15(128), to15(128), to15(128), MAX15]).unwrap();
        let d = Dither {
            mode: DitherMode::Picocad,
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
        // 128 is a hair closer to white, so white is the first color and the checker
        // starts with it.
        assert_eq!(&out.index[..4], &[1, 0, 1, 0]);
        assert_eq!(&out.index[4..8], &[0, 1, 0, 1]);
    }

    fn noise(w: u32, h: u32) -> Work {
        let mut img = Work::new(w, h, [0; 4]).unwrap();
        let mut s = 5u32;
        for (i, p) in img.px.iter_mut().enumerate() {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            let a = if i % 97 == 0 { 0 } else { 0x7f80 };
            *p = [
                (s & 0x7fff) as u16,
                (s >> 9 & 0x7fff) as u16,
                (s >> 17 & 0x7fff) as u16,
                a,
            ];
        }
        img
    }

    #[test]
    fn one_thread_and_many_agree_for_every_mode() {
        // 320x240 is above WAVE_FROM, so error diffusion takes the wavefront path.
        let img = noise(320, 240);
        let pal: Vec<[u8; 3]> = crate::palette::builtin("pico-8").unwrap().colors().to_vec();
        for mode in DitherMode::ALL {
            let d = Dither {
                mode,
                amount: 0.5,
                ..Dither::default()
            };
            let run = |t| {
                par::with_threads(t, || {
                    map(
                        &img,
                        &pal,
                        Metric::Redmean,
                        &d,
                        &Pool::new(),
                        &Cancel::new(),
                    )
                    .unwrap()
                })
            };
            assert_eq!(run(1), run(8), "{mode:?}");
            assert_eq!(run(3), run(8), "{mode:?}");
        }
    }

    #[test]
    fn cancelled_diffusion_stops() {
        let img = noise(400, 300);
        let pal = [[0u8; 3], [255; 3]];
        let d = Dither {
            mode: DitherMode::Stucki,
            ..Dither::default()
        };
        let c = Cancel::new();
        c.cancel();
        let r = par::with_threads(4, || map(&img, &pal, Metric::Rgb, &d, &Pool::new(), &c));
        assert!(matches!(r, Err(Error::Cancelled)));
    }

    #[test]
    fn bad_palettes_are_rejected() {
        let img = Work::new(1, 1, [0; 4]).unwrap();
        let d = Dither::default();
        assert!(map(&img, &[], Metric::Rgb, &d, &Pool::new(), &Cancel::new()).is_err());
    }
}
