//! Reducing (or enlarging) an image to the target size.
//!
//! The math follows the original program closely, including two details that look odd
//! but shape its results:
//!
//! - the source step is `(source size - 1) / target size`, not `source / target`, and
//!   each output pixel samples at `(x + offset + 0.5) * step`;
//! - interpolating filters treat pixels outside the image as opaque black instead of
//!   clamping to the edge, which darkens the last row and column a little.
//!
//! Matching both is what makes the golden tests pass.
//!
//! Speed comes from three things. Rows are independent and run in parallel
//! ([`crate::par`]). Everything that only depends on the column (source position,
//! fraction, Lanczos weights) is computed once per image instead of once per pixel. And
//! the four channels are processed as `[f32; 4]` with the same sequence of operations
//! as the scalar code, which the compiler turns into SSE instructions without changing
//! the result of a single channel.

use crate::buffer::{MAX15, Px, Work, to8, to15};
use crate::cancel::Cancel;
use crate::par;
use crate::pool::Pool;
use crate::quant::{self, Scratch};
use crate::settings::{Sample, SampleMode};
use crate::{Error, Result};

/// What the original uses for pixels outside the image: black with full alpha.
const OUTSIDE: Px = [0, 0, 0, MAX15];

type V4 = [f32; 4];

#[inline]
fn v4(p: Px) -> V4 {
    p.map(f32::from)
}

#[inline]
fn mul(a: V4, k: f32) -> V4 {
    a.map(|v| v * k)
}

#[inline]
fn add(a: V4, b: V4) -> V4 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]]
}

#[inline]
fn sub(a: V4, b: V4) -> V4 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2], a[3] - b[3]]
}

/// Where output pixels land in the source.
#[derive(Clone, Copy, Debug)]
struct Grid {
    step_x: f32,
    step_y: f32,
    off_x: f32,
    off_y: f32,
    rows: u32,
}

impl Grid {
    fn new(src: &Work, w: u32, h: u32, off_x: f32, off_y: f32) -> Self {
        Self {
            step_x: (src.width as f32 - 1.0) / w as f32,
            step_y: (src.height as f32 - 1.0) / h as f32,
            off_x,
            off_y,
            rows: h,
        }
    }

    #[inline]
    fn x(&self, x: u32) -> f32 {
        (x as f32 + self.off_x + 0.5) * self.step_x
    }

    #[inline]
    fn y(&self, y: u32) -> f32 {
        (y as f32 + self.off_y + 0.5) * self.step_y
    }
}

/// Per column values, computed once per image.
#[derive(Debug, Default)]
struct Cols {
    /// Source column of the sample, truncated like a C `(int)` cast.
    ix: Vec<i32>,
    /// Fraction between `ix` and the next column.
    fx: Vec<f32>,
    /// Lanczos weights, only filled for Lanczos.
    lw: Vec<[f32; 6]>,
}

impl Cols {
    fn new(g: &Grid, w: u32, lanczos: bool) -> Self {
        let mut c = Cols::default();
        for x in 0..w {
            let (i, f) = split(g.x(x));
            c.ix.push(i);
            c.fx.push(f);
            if lanczos {
                c.lw.push(lanczos_weights(f));
            }
        }
        c
    }
}

/// Integer part toward zero and the remainder, like a C `(int)` cast.
#[inline]
fn split(v: f32) -> (i32, f32) {
    let i = v as i32;
    (i, v - i as f32)
}

#[inline]
fn fetch(src: &Work, x: i32, y: i32) -> Px {
    if x >= 0 && y >= 0 && (x as u32) < src.width && (y as u32) < src.height {
        src.px[y as usize * src.width as usize + x as usize]
    } else {
        OUTSIDE
    }
}

/// `n` pixels of row `y` starting at column `x`, with [`OUTSIDE`] for pixels off the
/// image. Takes the fast path when the whole span is inside.
#[inline]
fn span<const N: usize>(src: &Work, x: i32, y: i32) -> [Px; N] {
    if y >= 0 && (y as u32) < src.height && x >= 0 && (x as usize + N) <= src.width as usize {
        let start = y as usize * src.width as usize + x as usize;
        src.px[start..start + N].try_into().unwrap_or([OUTSIDE; N])
    } else {
        std::array::from_fn(|i| fetch(src, x + i as i32, y))
    }
}

/// Clamps a filtered value the way the original does: truncate, then clamp.
#[inline]
fn store(v: f32) -> u16 {
    (v as i32).clamp(0, i32::from(MAX15)) as u16
}

#[inline]
fn store4(v: V4) -> Px {
    v.map(store)
}

fn row_nearest(src: &Work, g: &Grid, _: &Cols, y: u32, out: &mut [Px]) {
    let iy = (g.y(y).round() as i32).clamp(0, src.height as i32 - 1) as u32;
    let row = src.row(iy);
    let max_x = src.width as i32 - 1;
    for (x, o) in out.iter_mut().enumerate() {
        let ix = (g.x(x as u32).round() as i32).clamp(0, max_x);
        *o = row[ix as usize];
    }
}

/// Same operations as the original's `(1 - s) * a + s * b`, four channels at once.
#[inline]
fn lerp(a: V4, b: V4, s: f32) -> V4 {
    add(mul(a, 1.0 - s), mul(b, s))
}

fn row_bilinear(src: &Work, g: &Grid, cols: &Cols, y: u32, out: &mut [Px]) {
    let (iy, sy) = split(g.y(y));
    for (x, o) in out.iter_mut().enumerate() {
        let (ix, sx) = (cols.ix[x], cols.fx[x]);
        let top: [Px; 2] = span(src, ix, iy);
        let bottom: [Px; 2] = span(src, ix, iy + 1);
        let t0 = lerp(v4(top[0]), v4(top[1]), sx);
        let t1 = lerp(v4(bottom[0]), v4(bottom[1]), sx);
        *o = store4(lerp(t0, t1, sy));
    }
}

#[cfg(test)]
fn cubic(c: [f32; 4], t: f32) -> f32 {
    let r = cubic4(c.map(|v| [v; 4]), t);
    r[0]
}

/// Catmull-Rom (a = -0.5), written in the same order as the original so the f32
/// rounding comes out the same.
#[inline]
fn cubic4(c: [V4; 4], t: f32) -> V4 {
    let a0 = add(
        sub(add(mul(c[0], -0.5), mul(c[1], 1.5)), mul(c[2], 1.5)),
        mul(c[3], 0.5),
    );
    let a1 = sub(
        add(sub(c[0], mul(c[1], 2.5)), mul(c[2], 2.0)),
        mul(c[3], 0.5),
    );
    let a2 = add(mul(c[0], -0.5), mul(c[2], 0.5));
    let a3 = c[1];
    add(
        add(
            add(mul(mul(mul(a0, t), t), t), mul(mul(a1, t), t)),
            mul(a2, t),
        ),
        a3,
    )
}

fn row_bicubic(src: &Work, g: &Grid, cols: &Cols, y: u32, out: &mut [Px]) {
    let (iy, sy) = split(g.y(y));
    for (x, o) in out.iter_mut().enumerate() {
        let (ix, sx) = (cols.ix[x], cols.fx[x]);
        let rows: [V4; 4] = std::array::from_fn(|r| {
            let p: [Px; 4] = span(src, ix - 1, iy - 1 + r as i32);
            cubic4(p.map(v4), sx)
        });
        *o = store4(cubic4(rows, sy));
    }
}

/// Lanczos kernel with 3 lobes.
#[inline]
fn lanczos3(v: f32) -> f32 {
    // The original spells pi as 3.14159265359f, which rounds to the same f32 as PI.
    use std::f32::consts::PI;
    if v == 0.0 {
        return 1.0;
    }
    if !(-3.0..=3.0).contains(&v) {
        return 0.0;
    }
    (3.0 * (PI * v).sin() * (PI * v / 3.0).sin()) / (PI * PI * v * v)
}

fn lanczos_weights(s: f32) -> [f32; 6] {
    [s + 2.0, s + 1.0, s, s - 1.0, s - 2.0, s - 3.0].map(lanczos3)
}

/// `w[0] * v[0] + ... + w[5] * v[5]`, summed left to right like the original.
#[inline]
fn dot6(w: &[f32; 6], v: &[V4; 6]) -> V4 {
    let mut acc = mul(v[0], w[0]);
    for i in 1..6 {
        acc = add(acc, mul(v[i], w[i]));
    }
    acc
}

fn row_lanczos(src: &Work, g: &Grid, cols: &Cols, y: u32, out: &mut [Px]) {
    let (iy, sy) = split(g.y(y));
    let wy = lanczos_weights(sy);
    for (x, o) in out.iter_mut().enumerate() {
        let ix = cols.ix[x];
        let wx = &cols.lw[x];
        let acc: [V4; 6] = std::array::from_fn(|i| {
            let row: [Px; 6] = span(src, ix - 2, iy - 2 + i as i32);
            dot6(wx, &row.map(v4))
        });
        *o = store4(dot6(&wy, &acc));
    }
}

/// Averages every source pixel whose top left corner falls inside the output pixel.
/// Not in the original. It is what most people expect from "downscale" and the base of
/// the edge-preserving mode.
fn row_box(src: &Work, g: &Grid, _: &Cols, y: u32, out: &mut [Px]) {
    let w = out.len() as f32;
    let fx = src.width as f32 / w;
    let fy = src.height as f32 / g.rows as f32;
    let span = |start: f32, step: f32, len: u32| {
        let a = (start.floor().max(0.0) as u32).min(len - 1);
        let b = ((start + step).ceil().max(0.0) as u32).clamp(a + 1, len);
        (a, b)
    };
    let (y0, y1) = span((y as f32 + g.off_y) * fy, fy, src.height);
    for (x, o) in out.iter_mut().enumerate() {
        let (x0, x1) = span((x as f32 + g.off_x) * fx, fx, src.width);
        let mut sum = [0u64; 4];
        for sy in y0..y1 {
            for p in &src.row(sy)[x0 as usize..x1 as usize] {
                for c in 0..4 {
                    sum[c] += u64::from(p[c]);
                }
            }
        }
        let n = u64::from((x1 - x0) * (y1 - y0));
        *o = sum.map(|s| ((s + n / 2) / n) as u16);
    }
}

/// Fills one output row.
type RowFn = fn(&Work, &Grid, &Cols, u32, &mut [Px]);

fn row_fn(mode: SampleMode) -> RowFn {
    match mode {
        SampleMode::Nearest | SampleMode::Cluster => row_nearest,
        SampleMode::Bilinear => row_bilinear,
        SampleMode::Bicubic => row_bicubic,
        SampleMode::Lanczos => row_lanczos,
        SampleMode::Box => row_box,
    }
}

/// Cluster sampling, as in the original: for every output pixel, take the block of
/// source pixels it covers, run a 3-color k-means on it and keep the center of the
/// biggest cluster. Alpha comes from the nearest pixel. Falls back to nearest when the
/// output is larger than the source.
fn cluster(
    src: &Work,
    w: u32,
    h: u32,
    off: (f32, f32),
    out: &mut Work,
    cancel: &Cancel,
) -> Result<()> {
    let grid_x = src.width as f32 / w as f32;
    let grid_y = src.height as f32 / h as f32;
    let (gw, gh) = (grid_x as u32, grid_y as u32);
    if gw == 0 || gh == 0 {
        let near = Grid::new(src, w, h, off.0, off.1);
        let cols = Cols::default();
        return par::rows(&mut out.px, w as usize, 1, cancel, |y, line| {
            row_nearest(src, &near, &cols, y as u32, line);
        });
    }
    let max_x = src.width as i32 - 1;
    let max_y = src.height as i32 - 1;
    let init = || (Vec::with_capacity((gw * gh) as usize), Scratch::default());
    // k-means does 3 distance checks per pixel and pass, 8 passes, plus seeding.
    let cost = (gw * gh) as usize * 30;
    par::rows_with(
        &mut out.px,
        w as usize,
        cost,
        cancel,
        init,
        |(block, scratch), y, line| {
            let fy = y as f32;
            // Alpha uses a nearest pick with step source / target, unlike row_nearest.
            let iy = (((fy + off.1 + 0.5) * grid_y).round() as i32).clamp(0, max_y) as u32;
            for (x, o) in line.iter_mut().enumerate() {
                let fx = x as f32;
                let ix = (((fx + off.0 + 0.5) * grid_x).round() as i32).clamp(0, max_x) as u32;
                let alpha = src.at(ix, iy)[3];
                block.clear();
                for gy in 0..gh {
                    for gx in 0..gw {
                        let sx = fx * grid_x + gx as f32;
                        let sy = fy * grid_y + gy as f32;
                        let inside = sx >= 0.0
                            && sx < src.width as f32
                            && sy >= 0.0
                            && sy < src.height as f32;
                        block.push(if inside {
                            let p = src.at((fx * grid_x) as u32 + gx, (fy * grid_y) as u32 + gy);
                            [to8(p[0]), to8(p[1]), to8(p[2])]
                        } else {
                            [0; 3]
                        });
                    }
                }
                let c = quant::largest_cluster(block, 3, 0xdead_beef, scratch);
                *o = [to15(c[0]), to15(c[1]), to15(c[2]), alpha];
            }
        },
    )
}

/// Samples `src` to `w` x `h` with the given offsets (in output pixels).
///
/// # Errors
///
/// Fails when the source is empty or the run is cancelled.
pub fn sample(
    src: &Work,
    w: u32,
    h: u32,
    mode: SampleMode,
    offset: (f32, f32),
    pool: &Pool,
    cancel: &Cancel,
) -> Result<Work> {
    if src.width == 0 || src.height == 0 {
        return Err(Error::Invalid {
            name: "source",
            reason: "image is empty".into(),
        });
    }
    let (w, h) = (w.max(1), h.max(1));
    let mut out = pool.work(w, h, OUTSIDE);
    if mode == SampleMode::Cluster {
        cluster(src, w, h, offset, &mut out, cancel)?;
        return Ok(out);
    }
    let row = row_fn(mode);
    let grid = Grid::new(src, w, h, offset.0, offset.1);
    let cols = match mode {
        SampleMode::Bilinear | SampleMode::Bicubic | SampleMode::Lanczos => {
            Cols::new(&grid, w, mode == SampleMode::Lanczos)
        }
        _ => Cols::default(),
    };
    let cost = match mode {
        SampleMode::Nearest | SampleMode::Cluster => 1,
        SampleMode::Bilinear => 4,
        SampleMode::Bicubic => 16,
        SampleMode::Lanczos => 36,
        SampleMode::Box => (src.width / w).max(1) as usize * (src.height / h).max(1) as usize,
    };
    par::rows(&mut out.px, w as usize, cost, cancel, |y, line| {
        row(src, &grid, &cols, y as u32, line);
    })?;
    Ok(out)
}

/// Samples `src` with the sampling settings: target size (absolute, absolute keeping the
/// aspect ratio, or a divisor of the source size), mode and offsets.
///
/// # Errors
///
/// Fails when the source is empty, an offset is out of `-1..=1`, or the run is
/// cancelled.
pub fn run(src: &Work, s: &Sample, pool: &Pool, cancel: &Cancel) -> Result<Work> {
    for (v, name) in [(s.offset_x, "offset x"), (s.offset_y, "offset y")] {
        if !(v.is_finite() && (-1.0..=1.0).contains(&v)) {
            return Err(Error::Invalid {
                name,
                reason: "must be in -1..1".into(),
            });
        }
    }
    let (w, h) = s.size.resolve(src.width, src.height);
    sample(src, w, h, s.mode, (s.offset_x, s.offset_y), pool, cancel)
}

/// Whether a pixel counts as transparent for a given alpha threshold (`0..=255`). Same
/// test as the original: the 8-bit alpha, truncated, below the threshold.
#[inline]
#[must_use]
pub fn is_clear(p: Px, threshold: u8) -> bool {
    p[3] / 128 < u16::from(threshold)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::to15;

    fn ramp(w: u32, h: u32) -> Work {
        let mut img = Work::new(w, h, [0; 4]).unwrap();
        for y in 0..h {
            for x in 0..w {
                let i = img.idx(x, y);
                img.px[i] = [to15((x * 10) as u8), to15((y * 10) as u8), 0, MAX15];
            }
        }
        img
    }

    fn run(src: &Work, w: u32, h: u32, mode: SampleMode) -> Work {
        sample(src, w, h, mode, (0.0, 0.0), &Pool::new(), &Cancel::new()).unwrap()
    }

    #[test]
    fn same_size_nearest_picks_shifted_pixels() {
        // step is (4 - 1) / 4, so output x samples at (x + 0.5) * 0.75.
        let src = ramp(4, 1);
        let out = run(&src, 4, 1, SampleMode::Nearest);
        let xs: Vec<u16> = out.px.iter().map(|p| p[0] / to15(10)).collect();
        assert_eq!(xs, [0, 1, 2, 3]);
    }

    #[test]
    fn single_pixel_source() {
        let src = Work::new(1, 1, [100, 200, 300, MAX15]).unwrap();
        for mode in SampleMode::ALL {
            let out = run(&src, 3, 2, mode);
            assert_eq!(out.width, 3);
            assert_eq!(out.px[0][3], MAX15, "{mode:?}");
        }
        assert!(
            run(&src, 3, 2, SampleMode::Nearest)
                .px
                .iter()
                .all(|p| *p == src.px[0])
        );
    }

    #[test]
    fn bilinear_blends_neighbours() {
        let src = ramp(3, 1);
        let out = run(&src, 2, 1, SampleMode::Bilinear);
        // Step is (3 - 1) / 2 = 1, so x = 0 samples at 0.5, halfway between columns 0 and 1.
        assert_eq!(out.px[0][0], to15(10) / 2);
        // x = 1 samples at 1.5: columns 1 and 2.
        assert_eq!(out.px[1][0], u16::midpoint(to15(10), to15(20)));
    }

    #[test]
    fn kernels_hit_their_known_points() {
        assert_eq!(lanczos3(0.0), 1.0);
        assert!(lanczos3(1.0).abs() < 1e-6);
        assert!(lanczos3(2.0).abs() < 1e-6);
        assert_eq!(lanczos3(3.5), 0.0);
        assert!((lanczos3(0.5) - lanczos3(-0.5)).abs() < 1e-7);
        // Catmull-Rom passes through the middle samples.
        assert_eq!(cubic([1.0, 2.0, 3.0, 4.0], 0.0), 2.0);
        assert!((cubic([1.0, 2.0, 3.0, 4.0], 1.0) - 3.0).abs() < 1e-6);
        assert!((cubic([1.0, 2.0, 3.0, 4.0], 0.5) - 2.5).abs() < 1e-6);
    }

    #[test]
    fn flat_interior_stays_flat() {
        let src = Work::new(32, 32, [1000, 2000, 3000, MAX15]).unwrap();
        let p = run(&src, 8, 8, SampleMode::Bicubic).at(4, 4);
        for c in 0..3 {
            assert!(p[c].abs_diff(src.px[0][c]) <= 1, "{p:?}");
        }
        // The original does not normalize the Lanczos weights, and neither does xd-art
        // (that would break compatibility), so a flat area comes out slightly darker.
        let p = run(&src, 8, 8, SampleMode::Lanczos).at(4, 4);
        for c in 0..3 {
            let want = f32::from(src.px[0][c]);
            assert!((f32::from(p[c]) - want).abs() / want < 0.02, "{p:?}");
        }
    }

    #[test]
    fn box_averages_blocks() {
        let mut src = Work::new(4, 2, [0, 0, 0, MAX15]).unwrap();
        src.px[1] = [400, 0, 0, MAX15];
        src.px[5] = [400, 0, 0, MAX15];
        let out = run(&src, 2, 1, SampleMode::Box);
        assert_eq!(out.px[0][0], 200);
        assert_eq!(out.px[1][0], 0);
        let same = run(&src, 4, 2, SampleMode::Box);
        assert_eq!(same, src);
    }

    #[test]
    fn cluster_keeps_the_dominant_color() {
        let mut src = Work::new(8, 8, [to15(250), 0, 0, MAX15]).unwrap();
        src.px[0] = [0, 0, to15(250), MAX15];
        src.px[9] = [0, to15(250), 0, MAX15];
        let out = run(&src, 2, 2, SampleMode::Cluster);
        assert!(
            out.px.iter().all(|p| p[0] == to15(250) && p[1] == 0),
            "{:?}",
            out.px
        );
    }

    #[test]
    fn run_resolves_sizes() {
        use crate::settings::Size;
        let src = ramp(40, 20);
        let pool = Pool::new();
        let c = Cancel::new();
        let mut s = Sample {
            size: Size::Relative { x: 4, y: 5 },
            ..Sample::default()
        };
        let out = super::run(&src, &s, &pool, &c).unwrap();
        assert_eq!((out.width, out.height), (10, 4));
        s.size = Size::Absolute {
            width: 8,
            height: 99,
            keep_aspect: true,
        };
        let out = super::run(&src, &s, &pool, &c).unwrap();
        assert_eq!((out.width, out.height), (8, 4));
        s.offset_x = 1.5;
        assert!(super::run(&src, &s, &pool, &c).is_err());
    }

    #[test]
    fn offsets_move_the_sample_grid() {
        let src = ramp(16, 1);
        let pool = Pool::new();
        let c = Cancel::new();
        let at = |off: f32| {
            let s = Sample {
                size: crate::settings::Size::Absolute {
                    width: 4,
                    height: 1,
                    keep_aspect: false,
                },
                offset_x: off,
                ..Sample::default()
            };
            super::run(&src, &s, &pool, &c)
                .unwrap()
                .px
                .iter()
                .map(|p| p[0] / to15(10))
                .collect::<Vec<_>>()
        };
        assert_eq!(at(0.0), [2, 6, 9, 13]);
        assert_eq!(at(1.0), [6, 9, 13, 15]);
        assert_eq!(at(-1.0), [0, 2, 6, 9]);
    }

    #[test]
    fn alpha_threshold_matches_the_original_test() {
        assert!(is_clear([0, 0, 0, to15(127)], 128));
        assert!(!is_clear([0, 0, 0, to15(128)], 128));
        assert!(!is_clear([0, 0, 0, 0], 0));
        assert!(is_clear([0, 0, 0, to15(255) - 1], 255));
        assert!(!is_clear([0, 0, 0, to15(255)], 255));
    }

    #[test]
    fn one_thread_and_many_agree() {
        let mut src = Work::new(300, 200, [0; 4]).unwrap();
        let mut seed = 7u32;
        for p in &mut src.px {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            *p = [
                (seed & 0x7fff) as u16,
                (seed >> 8 & 0x7fff) as u16,
                (seed >> 16 & 0x7fff) as u16,
                MAX15,
            ];
        }
        for mode in SampleMode::ALL {
            let one = par::with_threads(1, || run(&src, 97, 61, mode));
            let many = par::with_threads(8, || run(&src, 97, 61, mode));
            assert_eq!(one, many, "{mode:?}");
        }
    }

    #[test]
    fn zero_size_is_clamped_and_empty_source_rejected() {
        let src = ramp(4, 4);
        assert_eq!(run(&src, 0, 0, SampleMode::Nearest).px.len(), 1);
        let empty = Work::empty();
        let r = sample(
            &empty,
            2,
            2,
            SampleMode::Nearest,
            (0.0, 0.0),
            &Pool::new(),
            &Cancel::new(),
        );
        assert!(r.is_err());
    }

    #[test]
    fn cancel_stops() {
        let c = Cancel::new();
        c.cancel();
        let r = sample(
            &ramp(4, 4),
            2,
            2,
            SampleMode::Nearest,
            (0.0, 0.0),
            &Pool::new(),
            &c,
        );
        assert!(matches!(r, Err(Error::Cancelled)));
    }
}
