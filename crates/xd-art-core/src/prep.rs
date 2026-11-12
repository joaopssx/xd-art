//! Blur on the source image, before sampling.
//!
//! Two implementations:
//!
//! - A separable gaussian with a normalized kernel and clamped edges. The kernel for a
//!   radius is built once and cached.
//! - The original program's blur: three passes of a box filter with a fractional radius,
//!   in fixed point. Repeated box passes approach a gaussian, and its radius `r` gives the
//!   same variance as a gaussian with `sigma = sqrt(r * (r + 1))`, which is the sigma the
//!   gaussian here uses. So the same radius means about the same blur in both.
//!
//! The box version is ported operation for operation, including the 32-bit sums, and
//! matches the original's output exactly.

use std::sync::{Arc, Mutex};

use crate::Result;
use crate::buffer::{Px, Work};
use crate::cancel::Cancel;
use crate::par;
use crate::pool::Pool;
use crate::settings::Blur;

/// Radii at or below this do nothing, as in the original.
const MIN_RADIUS: f32 = 0.01;

/// Gaussian sigma that matches the variance of three box passes of radius `r`.
#[must_use]
pub fn sigma_for(radius: f32) -> f32 {
    (radius * (radius + 1.0)).sqrt()
}

static KERNELS: Mutex<Vec<(u32, Arc<[f32]>)>> = Mutex::new(Vec::new());

/// Normalized gaussian weights from `-n` to `n`, where `n = ceil(3 sigma)`. Cached per
/// radius, keeping the last 16.
fn kernel(radius: f32) -> Arc<[f32]> {
    let key = radius.to_bits();
    let mut cache = KERNELS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(i) = cache.iter().position(|(k, _)| *k == key) {
        let hit = cache.remove(i);
        let k = Arc::clone(&hit.1);
        cache.push(hit);
        return k;
    }
    let sigma = sigma_for(radius);
    let n = (3.0 * sigma).ceil() as i32;
    let mut w: Vec<f32> = (-n..=n)
        .map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp())
        .collect();
    let total: f32 = w.iter().sum();
    for v in &mut w {
        *v /= total;
    }
    let k: Arc<[f32]> = w.into();
    if cache.len() >= 16 {
        cache.remove(0);
    }
    cache.push((key, Arc::clone(&k)));
    k
}

/// Blurs `img` in place. Does nothing (and touches no memory) when the radius is below
/// 0.01.
///
/// # Errors
///
/// Fails when the run is cancelled.
pub fn blur(img: &mut Work, b: &Blur, pool: &Pool, cancel: &Cancel) -> Result<()> {
    if b.radius <= MIN_RADIUS || img.px.is_empty() {
        return Ok(());
    }
    if b.box_approx {
        box_blur(img, b.radius, pool, cancel)
    } else {
        gaussian(img, b.radius, pool, cancel)
    }
}

/// Separable gaussian with clamped edges.
///
/// # Errors
///
/// Fails when the run is cancelled.
pub fn gaussian(img: &mut Work, radius: f32, pool: &Pool, cancel: &Cancel) -> Result<()> {
    let k = kernel(radius);
    let n = (k.len() / 2) as i32;
    let (w, h) = (img.width as usize, img.height as usize);
    let mut tmp = pool.px(w * h);
    tmp.resize(w * h, [0; 4]);
    {
        let src = &img.px;
        par::rows(&mut tmp, w, k.len(), cancel, |y, out| {
            let row = &src[y * w..(y + 1) * w];
            for (x, o) in out.iter_mut().enumerate() {
                let mut acc = [0.0f32; 4];
                for (i, &wt) in k.iter().enumerate() {
                    let sx = (x as i32 + i as i32 - n).clamp(0, w as i32 - 1) as usize;
                    let p = row[sx];
                    for c in 0..4 {
                        acc[c] += f32::from(p[c]) * wt;
                    }
                }
                *o = acc.map(|v| (v + 0.5) as u16);
            }
        })?;
    }
    let src = &tmp;
    par::rows(&mut img.px, w, k.len(), cancel, |y, out| {
        let mut acc = Vec::with_capacity(w);
        acc.resize(w, [0.0; 4]);
        for (i, &wt) in k.iter().enumerate() {
            let sy = (y as i32 + i as i32 - n).clamp(0, h as i32 - 1) as usize;
            for (a, p) in acc.iter_mut().zip(&src[sy * w..(sy + 1) * w]) {
                for c in 0..4 {
                    a[c] += f32::from(p[c]) * wt;
                }
            }
        }
        for (o, a) in out.iter_mut().zip(&acc) {
            *o = a.map(|v| (v + 0.5) as u16);
        }
    })?;
    pool.recycle_px(tmp);
    Ok(())
}

/// One pass of the original's fractional box filter over a line of pixels.
///
/// The arithmetic is the original's, in wrapping 32-bit integers like the C code. Where
/// the C code would read or write past the end of a line shorter than `2r + 2` pixels,
/// reads are clamped to the last pixel and extra writes are dropped.
fn box_line(src: &[Px], dst: &mut [Px], rad: f32) {
    let n = src.len();
    let r = rad as i32;
    let alpha = ((rad * 64.0) as i32) & 63;
    let alpha1 = 64 - alpha;
    let amp = (65536 * 64) / ((2 * r + 1) * 64 + alpha * 2).max(1);
    let at = |i: usize| src[i.min(n - 1)].map(i32::from);
    let first = src[0].map(i32::from);
    let last = src[n - 1].map(i32::from);
    let mut sum = [0i32; 4];
    for c in 0..4 {
        sum[c] = first[c]
            .wrapping_mul(alpha + alpha1)
            .wrapping_mul(r)
            .wrapping_add(first[c].wrapping_mul(alpha))
            .wrapping_add(first[c].wrapping_mul(alpha));
    }
    let mut put = |d: usize, sum: &[i32; 4]| {
        if d < n {
            dst[d] =
                sum.map(|s| ((i64::from(s) as u64).wrapping_mul(amp as u64) / (65536 * 64)) as u16);
        }
    };
    let add = |sum: &mut [i32; 4], s1: usize| {
        let (a, b) = (at(s1), at(s1 + 1));
        for c in 0..4 {
            sum[c] = sum[c].wrapping_add(
                a[c].wrapping_mul(alpha1)
                    .wrapping_add(b[c].wrapping_mul(alpha)),
            );
        }
    };
    let drop = |sum: &mut [i32; 4], s2: usize| {
        let (a, b) = (at(s2), at(s2 + 1));
        for c in 0..4 {
            sum[c] = sum[c].wrapping_sub(
                a[c].wrapping_mul(alpha)
                    .wrapping_add(b[c].wrapping_mul(alpha1)),
            );
        }
    };
    let mut s1 = 0;
    let mut d = 0;
    for _ in 0..r {
        add(&mut sum, s1);
        s1 += 1;
    }
    for _ in 0..=r {
        add(&mut sum, s1);
        s1 += 1;
        put(d, &sum);
        d += 1;
        for c in 0..4 {
            sum[c] = sum[c].wrapping_sub(first[c].wrapping_mul(alpha + alpha1));
        }
    }
    let mut s2 = 0;
    for _ in 0..(n as i32 - 2 * r - 2).max(0) {
        add(&mut sum, s1);
        s1 += 1;
        put(d, &sum);
        d += 1;
        drop(&mut sum, s2);
        s2 += 1;
    }
    for _ in 0..=r {
        for c in 0..4 {
            sum[c] = sum[c].wrapping_add(last[c].wrapping_mul(alpha1 + alpha));
        }
        put(d, &sum);
        d += 1;
        drop(&mut sum, s2);
        s2 += 1;
    }
}

/// Box passes over a line, in place, with two scratch lines. Rows get three passes and
/// columns two: the original runs three on columns too, but copies back the buffer that
/// holds the second one.
fn box_passes(line: &mut [Px], a: &mut Vec<Px>, b: &mut Vec<Px>, rad: f32, three: bool) {
    a.clear();
    a.resize(line.len(), [0; 4]);
    b.clear();
    b.resize(line.len(), [0; 4]);
    box_line(line, a, rad);
    box_line(a, b, rad);
    if three {
        box_line(b, line, rad);
    } else {
        line.copy_from_slice(b);
    }
}

/// The original's blur: three box passes on every row, then two on every column.
///
/// # Errors
///
/// Fails when the run is cancelled.
pub fn box_blur(img: &mut Work, radius: f32, pool: &Pool, cancel: &Cancel) -> Result<()> {
    let (w, h) = (img.width as usize, img.height as usize);
    // The original skips images not bigger than the radius.
    if radius <= MIN_RADIUS || w <= radius as usize || h <= radius as usize {
        return Ok(());
    }
    let init = || (Vec::new(), Vec::new());
    par::rows_with(&mut img.px, w, 6, cancel, init, |(a, b), _, row| {
        box_passes(row, a, b, radius, true);
    })?;
    // Columns: transpose, blur the rows of the transposed image, transpose back.
    let mut t = pool.px(w * h);
    t.resize(w * h, [0; 4]);
    transpose(&img.px, &mut t, w, h);
    par::rows_with(&mut t, h, 6, cancel, init, |(a, b), _, row| {
        box_passes(row, a, b, radius, false);
    })?;
    transpose(&t, &mut img.px, h, w);
    pool.recycle_px(t);
    Ok(())
}

/// Writes the `w` x `h` image `src` transposed into `dst` (`h` x `w`), in tiles so both
/// sides stay in cache.
fn transpose(src: &[Px], dst: &mut [Px], w: usize, h: usize) {
    const T: usize = 32;
    for by in (0..h).step_by(T) {
        for bx in (0..w).step_by(T) {
            for y in by..(by + T).min(h) {
                for x in bx..(bx + T).min(w) {
                    dst[x * h + y] = src[y * w + x];
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::MAX15;

    fn noise(w: u32, h: u32) -> Work {
        let mut img = Work::new(w, h, [0; 4]).unwrap();
        let mut s = 99u32;
        for p in &mut img.px {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            *p = [
                (s & 0x7fff) as u16,
                (s >> 8 & 0x7fff) as u16,
                (s >> 16 & 0x7fff) as u16,
                MAX15,
            ];
        }
        img
    }

    fn naive(img: &Work, radius: f32) -> Work {
        let k = kernel(radius);
        let n = (k.len() / 2) as i32;
        let (w, h) = (img.width as i32, img.height as i32);
        let mut out = img.clone();
        for y in 0..h {
            for x in 0..w {
                let mut acc = [0.0f64; 4];
                for (j, wy) in k.iter().enumerate() {
                    for (i, wx) in k.iter().enumerate() {
                        let sx = (x + i as i32 - n).clamp(0, w - 1) as u32;
                        let sy = (y + j as i32 - n).clamp(0, h - 1) as u32;
                        let p = img.at(sx, sy);
                        for c in 0..4 {
                            acc[c] += f64::from(p[c]) * f64::from(*wx) * f64::from(*wy);
                        }
                    }
                }
                let i = out.idx(x as u32, y as u32);
                out.px[i] = acc.map(|v| v.round() as u16);
            }
        }
        out
    }

    #[test]
    fn separable_matches_naive_2d() {
        for radius in [0.5, 1.0, 2.7] {
            let src = noise(23, 17);
            let mut fast = src.clone();
            gaussian(&mut fast, radius, &Pool::new(), &Cancel::new()).unwrap();
            let slow = naive(&src, radius);
            let max = fast
                .px
                .iter()
                .zip(&slow.px)
                .flat_map(|(a, b)| (0..4).map(move |c| a[c].abs_diff(b[c])))
                .max()
                .unwrap();
            // Two f32 passes with rounding in between against one f64 pass.
            assert!(max <= 2, "radius {radius}: max diff {max}");
        }
    }

    #[test]
    fn kernel_is_normalized_and_cached() {
        let k = kernel(3.0);
        assert!((k.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        assert_eq!(k.len() % 2, 1);
        assert!(Arc::ptr_eq(&k, &kernel(3.0)));
    }

    #[test]
    fn zero_radius_touches_nothing() {
        let pool = Pool::new();
        let mut img = noise(8, 8);
        let before = img.clone();
        for box_approx in [false, true] {
            blur(
                &mut img,
                &Blur {
                    radius: 0.0,
                    box_approx,
                },
                &pool,
                &Cancel::new(),
            )
            .unwrap();
        }
        assert_eq!(img, before);
        assert_eq!(pool.held(), 0);
    }

    #[test]
    fn flat_stays_flat() {
        let flat = Work::new(40, 30, [1234, 5678, 9012, MAX15]).unwrap();
        for box_approx in [false, true] {
            let mut img = flat.clone();
            blur(
                &mut img,
                &Blur {
                    radius: 3.5,
                    box_approx,
                },
                &Pool::new(),
                &Cancel::new(),
            )
            .unwrap();
            for p in &img.px {
                for c in 0..4 {
                    assert!(p[c].abs_diff(flat.px[0][c]) <= 2, "{box_approx} {p:?}");
                }
            }
        }
    }

    #[test]
    fn short_lines_do_not_overrun() {
        let mut img = noise(5, 40);
        box_blur(&mut img, 3.0, &Pool::new(), &Cancel::new()).unwrap();
        let mut img = noise(3, 3);
        box_blur(&mut img, 4.0, &Pool::new(), &Cancel::new()).unwrap();
    }

    #[test]
    fn one_thread_and_many_agree() {
        let src = noise(300, 120);
        for box_approx in [false, true] {
            let b = Blur {
                radius: 2.5,
                box_approx,
            };
            let run = |t| {
                par::with_threads(t, || {
                    let mut img = src.clone();
                    blur(&mut img, &b, &Pool::new(), &Cancel::new()).unwrap();
                    img
                })
            };
            assert_eq!(run(1), run(8));
        }
    }

    #[test]
    fn sigma_matches_box_variance() {
        // Variance of one box of width 2r+1 is r(r+1)/3; three passes give r(r+1).
        assert!((sigma_for(2.0) - 6f32.sqrt()).abs() < 1e-6);
    }
}
