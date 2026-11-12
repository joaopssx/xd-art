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

/// Fills one output row.
type RowFn = fn(&Work, &Grid, u32, &mut [Px]);

fn row_fn(mode: SampleMode) -> Result<RowFn> {
    match mode {
        SampleMode::Nearest => Ok(row_nearest),
        SampleMode::Bilinear => Ok(row_bilinear),
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
        for mode in [SampleMode::Nearest, SampleMode::Bilinear] {
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
