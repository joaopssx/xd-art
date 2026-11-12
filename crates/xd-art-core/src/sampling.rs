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
//! Matching both is what makes the golden tests pass. Every function works one output
//! row at a time so rows can run in parallel.

use crate::buffer::{MAX15, Px, Work};
use crate::cancel::Cancel;
use crate::pool::Pool;
use crate::settings::SampleMode;
use crate::{Error, Result};

/// What the original uses for pixels outside the image: black with full alpha.
const OUTSIDE: Px = [0, 0, 0, MAX15];

/// Where output pixels land in the source.
#[derive(Clone, Copy, Debug)]
struct Grid {
    step_x: f32,
    step_y: f32,
    off_x: f32,
    off_y: f32,
}

impl Grid {
    fn new(src: &Work, w: u32, h: u32, off_x: f32, off_y: f32) -> Self {
        Self {
            step_x: (src.width as f32 - 1.0) / w as f32,
            step_y: (src.height as f32 - 1.0) / h as f32,
            off_x,
            off_y,
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

/// Integer part toward zero and the remainder, like a C `(int)` cast.
#[inline]
fn split(v: f32) -> (i32, f32) {
    let i = v as i32;
    (i, v - i as f32)
}

#[inline]
fn fetch(src: &Work, x: i32, y: i32) -> Px {
    if x >= 0 && y >= 0 && (x as u32) < src.width && (y as u32) < src.height {
        src.at(x as u32, y as u32)
    } else {
        OUTSIDE
    }
}

/// Clamps a filtered value the way the original does: truncate, then clamp.
#[inline]
fn store(v: f32) -> u16 {
    (v as i32).clamp(0, i32::from(MAX15)) as u16
}

fn row_nearest(src: &Work, g: &Grid, y: u32, out: &mut [Px]) {
    let iy = (g.y(y).round() as i32).clamp(0, src.height as i32 - 1) as u32;
    let row = src.row(iy);
    let max_x = src.width as i32 - 1;
    for (x, o) in out.iter_mut().enumerate() {
        let ix = (g.x(x as u32).round() as i32).clamp(0, max_x);
        *o = row[ix as usize];
    }
}

#[inline]
fn lerp2(sx: f32, sy: f32, c: [f32; 4]) -> f32 {
    let t0 = (1.0 - sx) * c[0] + sx * c[1];
    let t1 = (1.0 - sx) * c[2] + sx * c[3];
    (1.0 - sy) * t0 + sy * t1
}

fn row_bilinear(src: &Work, g: &Grid, y: u32, out: &mut [Px]) {
    let (iy, sy) = split(g.y(y));
    for (x, o) in out.iter_mut().enumerate() {
        let (ix, sx) = split(g.x(x as u32));
        let p = [
            fetch(src, ix, iy),
            fetch(src, ix + 1, iy),
            fetch(src, ix, iy + 1),
            fetch(src, ix + 1, iy + 1),
        ];
        for c in 0..4 {
            let v = [p[0][c], p[1][c], p[2][c], p[3][c]].map(f32::from);
            o[c] = store(lerp2(sx, sy, v));
        }
    }
}

#[inline]
fn cubic(c: [f32; 4], t: f32) -> f32 {
    // Catmull-Rom (a = -0.5), written in the same order as the original so the f32
    // rounding comes out the same.
    let a0 = -0.5 * c[0] + 1.5 * c[1] - 1.5 * c[2] + 0.5 * c[3];
    let a1 = c[0] - 2.5 * c[1] + 2.0 * c[2] - 0.5 * c[3];
    let a2 = -0.5 * c[0] + 0.5 * c[2];
    let a3 = c[1];
    a0 * t * t * t + a1 * t * t + a2 * t + a3
}

fn row_bicubic(src: &Work, g: &Grid, y: u32, out: &mut [Px]) {
    let (iy, sy) = split(g.y(y));
    for (x, o) in out.iter_mut().enumerate() {
        let (ix, sx) = split(g.x(x as u32));
        let mut p = [[OUTSIDE; 4]; 4];
        for (r, row) in p.iter_mut().enumerate() {
            for (c, px) in row.iter_mut().enumerate() {
                *px = fetch(src, ix - 1 + c as i32, iy - 1 + r as i32);
            }
        }
        for ch in 0..4 {
            let rows = p.map(|row| cubic(row.map(|px| f32::from(px[ch])), sx));
            o[ch] = store(cubic(rows, sy));
        }
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

#[inline]
fn dot6(w: &[f32; 6], v: [f32; 6]) -> f32 {
    w[0] * v[0] + w[1] * v[1] + w[2] * v[2] + w[3] * v[3] + w[4] * v[4] + w[5] * v[5]
}

fn row_lanczos(src: &Work, g: &Grid, y: u32, out: &mut [Px]) {
    let (iy, sy) = split(g.y(y));
    let wy = [sy + 2.0, sy + 1.0, sy, sy - 1.0, sy - 2.0, sy - 3.0].map(lanczos3);
    for (x, o) in out.iter_mut().enumerate() {
        let (ix, sx) = split(g.x(x as u32));
        let wx = [sx + 2.0, sx + 1.0, sx, sx - 1.0, sx - 2.0, sx - 3.0].map(lanczos3);
        let mut acc = [[0.0f32; 6]; 4];
        for i in 0..6 {
            let row: [Px; 6] =
                std::array::from_fn(|c| fetch(src, ix - 2 + c as i32, iy - 2 + i as i32));
            for (ch, a) in acc.iter_mut().enumerate() {
                a[i] = dot6(&wx, row.map(|px| f32::from(px[ch])));
            }
        }
        for ch in 0..4 {
            o[ch] = store(dot6(&wy, acc[ch]));
        }
    }
}

/// Fills one output row.
type RowFn = fn(&Work, &Grid, u32, &mut [Px]);

fn row_fn(mode: SampleMode) -> Result<RowFn> {
    match mode {
        SampleMode::Nearest => Ok(row_nearest),
        SampleMode::Bilinear => Ok(row_bilinear),
        SampleMode::Bicubic => Ok(row_bicubic),
        SampleMode::Lanczos => Ok(row_lanczos),
        other => Err(Error::Unsupported(format!(
            "sampling mode {}",
            other.name()
        ))),
    }
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
    let row = row_fn(mode)?;
    let grid = Grid::new(src, w, h, offset.0, offset.1);
    let mut out = pool.work(w, h, OUTSIDE);
    for (y, line) in out.px.chunks_exact_mut(w as usize).enumerate() {
        cancel.check()?;
        row(src, &grid, y as u32, line);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)] // kernel values at exact points
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
        for mode in [
            SampleMode::Nearest,
            SampleMode::Bilinear,
            SampleMode::Bicubic,
            SampleMode::Lanczos,
        ] {
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
