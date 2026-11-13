//! Exports: sprite sheets, and effects applied on the way out.
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

/// Layout of a sprite sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SheetLayout {
    /// Frames per row. 0 puts every frame on one row (a horizontal strip); 1 makes a
    /// vertical strip.
    pub columns: u32,
    /// Transparent pixels between frames and around the sheet.
    pub spacing: u32,
}

/// Where each frame of a sheet is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SheetFrame {
    /// Left edge in the sheet.
    pub x: u32,
    /// Top edge in the sheet.
    pub y: u32,
    /// Display time in milliseconds, 0 for still images.
    pub duration_ms: u32,
}

/// A sprite sheet and its frame positions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sheet {
    /// The packed image.
    pub image: Image,
    /// Width of every frame.
    pub frame_width: u32,
    /// Height of every frame.
    pub frame_height: u32,
    /// Columns used.
    pub columns: u32,
    /// Rows used.
    pub rows: u32,
    /// Spacing used.
    pub spacing: u32,
    /// Frame positions, in input order.
    pub frames: Vec<SheetFrame>,
}

/// Packs frames of the same size into a grid, left to right, top to bottom.
/// `durations_ms` may be empty (still frames) or have one entry per frame.
///
/// # Errors
///
/// Fails when there are no frames, the frames differ in size, or the sheet would be
/// too large.
pub fn sprite_sheet(frames: &[Image], durations_ms: &[u32], layout: SheetLayout) -> Result<Sheet> {
    let first = frames.first().ok_or_else(|| Error::Invalid {
        name: "sprite sheet",
        reason: "no frames".into(),
    })?;
    let (fw, fh) = (first.width(), first.height());
    if frames.iter().any(|f| (f.width(), f.height()) != (fw, fh)) {
        return Err(Error::Invalid {
            name: "sprite sheet",
            reason: "frames have different sizes".into(),
        });
    }
    let n = frames.len() as u32;
    let columns = if layout.columns == 0 {
        n
    } else {
        layout.columns.min(n)
    };
    let rows = n.div_ceil(columns);
    let sp = layout.spacing;
    let w = columns.checked_mul(fw + sp).and_then(|v| v.checked_add(sp));
    let h = rows.checked_mul(fh + sp).and_then(|v| v.checked_add(sp));
    let (Some(w), Some(h)) = (w, h) else {
        return Err(Error::Invalid {
            name: "sprite sheet",
            reason: "too large".into(),
        });
    };
    let mut rgba = vec![0u8; w as usize * h as usize * 4];
    let mut placed = Vec::with_capacity(frames.len());
    for (i, f) in frames.iter().enumerate() {
        let (c, r) = (i as u32 % columns, i as u32 / columns);
        let (x0, y0) = (sp + c * (fw + sp), sp + r * (fh + sp));
        let src = f.to_rgba();
        for y in 0..fh {
            let s = (y * fw * 4) as usize;
            let d = (((y0 + y) * w + x0) * 4) as usize;
            rgba[d..d + (fw * 4) as usize].copy_from_slice(&src[s..s + (fw * 4) as usize]);
        }
        placed.push(SheetFrame {
            x: x0,
            y: y0,
            duration_ms: durations_ms.get(i).copied().unwrap_or(0),
        });
    }
    Ok(Sheet {
        image: Image::from_rgba(w, h, &rgba)?,
        frame_width: fw,
        frame_height: fh,
        columns,
        rows,
        spacing: sp,
        frames: placed,
    })
}

impl Sheet {
    /// Frame positions as JSON. The format is xd-art's own, documented in
    /// `docs/integration.md`; it is plain enough to read from any engine's scripting
    /// language.
    #[cfg(feature = "serde")]
    #[must_use]
    pub fn to_json(&self, image_file: &str) -> String {
        let frames: Vec<serde_json::Value> = self
            .frames
            .iter()
            .map(|f| {
                serde_json::json!({
                    "x": f.x, "y": f.y, "w": self.frame_width, "h": self.frame_height,
                    "duration_ms": f.duration_ms,
                })
            })
            .collect();
        let v = serde_json::json!({
            "xd-art-sheet": 1,
            "image": image_file,
            "frame_width": self.frame_width,
            "frame_height": self.frame_height,
            "columns": self.columns,
            "rows": self.rows,
            "spacing": self.spacing,
            "frames": frames,
        });
        serde_json::to_string_pretty(&v).unwrap_or_default()
    }
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

    fn frames(n: u8) -> Vec<Image> {
        (0..n)
            .map(|i| Image::new(3, 2, [i * 10, 0, 0]).unwrap())
            .collect()
    }

    #[test]
    fn sheet_layouts() {
        let f = frames(5);
        let strip = sprite_sheet(&f, &[], SheetLayout::default()).unwrap();
        assert_eq!((strip.image.width(), strip.image.height()), (15, 2));
        let vertical = sprite_sheet(
            &f,
            &[],
            SheetLayout {
                columns: 1,
                spacing: 0,
            },
        )
        .unwrap();
        assert_eq!((vertical.image.width(), vertical.image.height()), (3, 10));
        let grid = sprite_sheet(
            &f,
            &[100; 5],
            SheetLayout {
                columns: 2,
                spacing: 1,
            },
        )
        .unwrap();
        assert_eq!((grid.columns, grid.rows), (2, 3));
        assert_eq!(
            (grid.image.width(), grid.image.height()),
            (2 * 4 + 1, 3 * 3 + 1)
        );
        assert_eq!(
            grid.frames[3],
            SheetFrame {
                x: 5,
                y: 4,
                duration_ms: 100
            }
        );
        assert_eq!(grid.image.get(5, 4), Some([30, 0, 0, 255]));
        assert_eq!(
            grid.image.get(0, 0),
            Some([0, 0, 0, 0]),
            "spacing is transparent"
        );
    }

    #[test]
    fn sheet_rejects_mixed_sizes() {
        let mut f = frames(2);
        f.push(Image::new(4, 4, [0; 3]).unwrap());
        assert!(sprite_sheet(&f, &[], SheetLayout::default()).is_err());
        assert!(sprite_sheet(&[], &[], SheetLayout::default()).is_err());
    }

    #[cfg(feature = "serde")]
    #[test]
    fn sheet_json_lists_frames() {
        let s = sprite_sheet(&frames(2), &[80, 120], SheetLayout::default()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&s.to_json("hero.png")).unwrap();
        assert_eq!(v["xd-art-sheet"], 1);
        assert_eq!(v["frames"][1]["x"], 3);
        assert_eq!(v["frames"][1]["duration_ms"], 120);
        assert_eq!(v["image"], "hero.png");
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
