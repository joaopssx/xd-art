//! Exports: effects applied on the way out.
//!
//! Effects are not part of the conversion. They run on the finished, upscaled image
//! when saving, are all off by default, and never change what the preview shows unless
//! the interface asks for them.

use crate::buffer::{Image, Indexed};
use crate::{Error, Result};

/// Effects for an export. The default does nothing.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Effects {
    /// Darkens the last row of every scaled pixel row, `0..=1`. Needs an output scale
    /// of 2 or more to have rows to darken.
    pub scanlines: f32,
    /// Barrel distortion of a curved CRT screen with darker corners, `0..=1`. Small
    /// values (0.1 to 0.2) look like a real tube.
    pub crt: f32,
}

impl Effects {
    /// Whether any effect is on.
    #[must_use]
    pub fn any(&self) -> bool {
        self.scanlines > 0.0 || self.crt > 0.0
    }

    /// Applies the effects to an image upscaled by `scale`.
    ///
    /// # Errors
    ///
    /// Fails when a value is out of range.
    pub fn apply(&self, img: &Image, scale: u32) -> Result<Image> {
        for (v, name) in [(self.scanlines, "scanlines"), (self.crt, "crt")] {
            if !(v.is_finite() && (0.0..=1.0).contains(&v)) {
                return Err(Error::Invalid {
                    name,
                    reason: "must be in 0..1".into(),
                });
            }
        }
        let mut out = img.clone();
        if self.scanlines > 0.0 {
            out = scanlines(&out, scale, self.scanlines);
        }
        if self.crt > 0.0 {
            out = crt(&out, self.crt);
        }
        Ok(out)
    }
}

/// Darkens the last row of each block of `scale` rows by `strength`.
#[must_use]
pub fn scanlines(img: &Image, scale: u32, strength: f32) -> Image {
    let mut out = img.clone();
    if scale < 2 {
        return out;
    }
    let keep = 1.0 - strength.clamp(0.0, 1.0);
    for y in (scale - 1..img.height()).step_by(scale as usize) {
        for x in 0..img.width() {
            if let Some(p) = img.get(x, y) {
                let d = |v: u8| (f32::from(v) * keep).round() as u8;
                out.set(x, y, [d(p[0]), d(p[1]), d(p[2]), p[3]]);
            }
        }
    }
    out
}

/// Barrel distortion with nearest sampling and a vignette. Pixels that fall outside the
/// source after the distortion are black.
#[must_use]
pub fn crt(img: &Image, amount: f32) -> Image {
    let (w, h) = (img.width() as f32, img.height() as f32);
    let k = amount.clamp(0.0, 1.0) * 0.25;
    let mut out = img.clone();
    for y in 0..img.height() {
        for x in 0..img.width() {
            // -1..1 around the center.
            let u = (x as f32 + 0.5) / w * 2.0 - 1.0;
            let v = (y as f32 + 0.5) / h * 2.0 - 1.0;
            let r2 = u * u + v * v;
            let (su, sv) = (u * (1.0 + k * r2), v * (1.0 + k * r2));
            let p = if su.abs() > 1.0 || sv.abs() > 1.0 {
                [0, 0, 0, 255]
            } else {
                let sx = (((su * 0.5 + 0.5) * w) as u32).min(img.width() - 1);
                let sy = (((sv * 0.5 + 0.5) * h) as u32).min(img.height() - 1);
                let p = img.get(sx, sy).unwrap_or([0, 0, 0, 255]);
                let shade = 1.0 - amount * 0.35 * r2 / 2.0;
                let d = |c: u8| (f32::from(c) * shade).round() as u8;
                [d(p[0]), d(p[1]), d(p[2]), p[3]]
            };
            out.set(x, y, p);
        }
    }
    out
}

/// Palette cycling: frames where the colors `first..=last` of the palette rotate by
/// one step each frame, the old way of animating water and fire. Returns the palette
/// of every frame; the indices stay the same, so they can go straight to
/// [`crate::anim::encode_gif`].
///
/// # Errors
///
/// Fails when the range is empty or outside the palette.
pub fn cycle_palettes(palette: &[[u8; 3]], first: usize, last: usize) -> Result<Vec<Vec<[u8; 3]>>> {
    if first >= last || last >= palette.len() {
        return Err(Error::Invalid {
            name: "cycle range",
            reason: format!(
                "{first}..{last} does not fit a palette of {}",
                palette.len()
            ),
        });
    }
    let n = last - first + 1;
    Ok((0..n)
        .map(|step| {
            let mut p = palette.to_vec();
            for i in 0..n {
                p[first + (i + step) % n] = palette[first + i];
            }
            p
        })
        .collect())
}

/// Expands one indexed image with each palette of a cycle.
///
/// # Errors
///
/// Fails when the image is empty.
pub fn cycle_frames(ix: &Indexed, palettes: &[Vec<[u8; 3]>]) -> Result<Vec<Image>> {
    palettes.iter().map(|p| ix.to_image(p)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_effects_do_nothing() {
        let img = Image::new(4, 4, [100, 150, 200]).unwrap();
        let e = Effects::default();
        assert!(!e.any());
        assert_eq!(e.apply(&img, 2).unwrap(), img);
        assert!(Effects { crt: 2.0, ..e }.apply(&img, 1).is_err());
    }

    #[test]
    fn scanlines_darken_one_row_per_pixel_row() {
        let img = Image::new(2, 4, [200, 200, 200]).unwrap();
        let s = scanlines(&img, 2, 0.5);
        assert_eq!(s.get(0, 0), Some([200, 200, 200, 255]));
        assert_eq!(s.get(0, 1), Some([100, 100, 100, 255]));
        assert_eq!(s.get(1, 3), Some([100, 100, 100, 255]));
        assert_eq!(scanlines(&img, 1, 1.0), img);
    }

    #[test]
    fn crt_keeps_the_center_and_darkens_corners() {
        let img = Image::new(33, 33, [200, 200, 200]).unwrap();
        let c = crt(&img, 0.5);
        assert_eq!(c.get(16, 16), Some([200, 200, 200, 255]));
        let corner = c.get(0, 0).unwrap();
        assert!(corner[0] < 200, "{corner:?}");
    }

    #[test]
    fn cycling_rotates_only_the_range() {
        let pal = [[0; 3], [1; 3], [2; 3], [3; 3], [9; 3]];
        let frames = cycle_palettes(&pal, 1, 3).unwrap();
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0], pal.to_vec());
        assert_eq!(frames[1], vec![[0; 3], [3; 3], [1; 3], [2; 3], [9; 3]]);
        assert!(cycle_palettes(&pal, 3, 3).is_err());
        assert!(cycle_palettes(&pal, 1, 5).is_err());
        let ix = Indexed::new(1, 1, 1).unwrap();
        let imgs = cycle_frames(&ix, &frames).unwrap();
        assert_eq!(imgs[1].get(0, 0), Some([3, 3, 3, 255]));
    }
}
