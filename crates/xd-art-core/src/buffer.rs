//! Pixel buffers.
//!
//! [`Image`] is what goes in and out of the engine: 8-bit RGB packed three bytes per
//! pixel, with alpha in a separate plane that only exists when the image actually has
//! transparency. A 8000x6000 photo without alpha costs 144 MB this way instead of 192 MB
//! as RGBA.
//!
//! [`Work`] is the format the stages pass between each other: 15 bits per channel,
//! stored as `[u16; 4]`. It is the same scale the original program uses (an 8-bit value
//! shifted left by 7), which keeps the integer math of the ported algorithms identical.
//! Working images are small most of the time, since sampling runs early.
//!
//! [`Indexed`] is the result of mapping a working image to a palette.

use crate::{Error, Result};

/// Largest value of a channel in the working format.
pub const MAX15: u16 = 0x7fff;

/// Alpha of an opaque 8-bit pixel in the working format (`255 << 7`). The original
/// converts this way too, so an opaque pixel is not quite [`MAX15`].
pub const OPAQUE: u16 = to15(255);

/// One pixel of the working format: red, green, blue, alpha, each `0..=MAX15`.
pub type Px = [u16; 4];

/// Converts an 8-bit channel to the working scale.
#[inline]
#[must_use]
pub const fn to15(v: u8) -> u16 {
    (v as u16) << 7
}

/// Converts a working channel back to 8 bits. Values above [`MAX15`] saturate.
#[inline]
#[must_use]
pub const fn to8(v: u16) -> u8 {
    if v >= MAX15 { 255 } else { (v >> 7) as u8 }
}

fn checked_len(width: u32, height: u32, per_pixel: usize) -> Result<usize> {
    if width == 0 || height == 0 {
        return Err(Error::Invalid {
            name: "size",
            reason: format!("{width}x{height} has no pixels"),
        });
    }
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(per_pixel))
        .ok_or_else(|| Error::Invalid {
            name: "size",
            reason: format!("{width}x{height} is too large"),
        })
}

/// An 8-bit RGB image with optional alpha.
#[derive(Clone, PartialEq, Eq)]
pub struct Image {
    width: u32,
    height: u32,
    rgb: Vec<u8>,
    alpha: Option<Vec<u8>>,
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Image")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("alpha", &self.alpha.is_some())
            .finish_non_exhaustive()
    }
}

impl Image {
    /// Creates an opaque image filled with one color.
    ///
    /// # Errors
    ///
    /// Fails when a side is zero or the size does not fit in memory.
    pub fn new(width: u32, height: u32, rgb: [u8; 3]) -> Result<Self> {
        let len = checked_len(width, height, 3)?;
        let mut data = Vec::with_capacity(len);
        for _ in 0..len / 3 {
            data.extend_from_slice(&rgb);
        }
        Ok(Self {
            width,
            height,
            rgb: data,
            alpha: None,
        })
    }

    /// Wraps packed RGB bytes, and an alpha plane if there is one.
    ///
    /// # Errors
    ///
    /// Fails when the buffers do not match the size.
    pub fn from_parts(
        width: u32,
        height: u32,
        rgb: Vec<u8>,
        alpha: Option<Vec<u8>>,
    ) -> Result<Self> {
        let len = checked_len(width, height, 3)?;
        if rgb.len() != len {
            return Err(Error::Invalid {
                name: "rgb buffer",
                reason: format!("expected {len} bytes, got {}", rgb.len()),
            });
        }
        if let Some(a) = &alpha
            && a.len() != len / 3
        {
            return Err(Error::Invalid {
                name: "alpha buffer",
                reason: format!("expected {} bytes, got {}", len / 3, a.len()),
            });
        }
        Ok(Self {
            width,
            height,
            rgb,
            alpha,
        })
    }

    /// Builds an image from interleaved RGBA bytes. The alpha plane is dropped when
    /// every pixel is opaque.
    ///
    /// # Errors
    ///
    /// Fails when the buffer does not match the size.
    pub fn from_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<Self> {
        let len = checked_len(width, height, 4)?;
        if rgba.len() != len {
            return Err(Error::Invalid {
                name: "rgba buffer",
                reason: format!("expected {len} bytes, got {}", rgba.len()),
            });
        }
        let mut rgb = Vec::with_capacity(len / 4 * 3);
        let mut alpha = Vec::with_capacity(len / 4);
        for p in rgba.chunks_exact(4) {
            rgb.extend_from_slice(&p[..3]);
            alpha.push(p[3]);
        }
        let alpha = alpha.iter().any(|&a| a != 255).then_some(alpha);
        Ok(Self {
            width,
            height,
            rgb,
            alpha,
        })
    }

    /// Width in pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Packed RGB bytes, row by row.
    #[must_use]
    pub fn rgb(&self) -> &[u8] {
        &self.rgb
    }

    /// The alpha plane, one byte per pixel, or `None` when the image is opaque.
    #[must_use]
    pub fn alpha(&self) -> Option<&[u8]> {
        self.alpha.as_deref()
    }

    /// One row of packed RGB bytes. Panics if `y` is out of range.
    #[must_use]
    pub fn row(&self, y: u32) -> &[u8] {
        let w = self.width as usize * 3;
        &self.rgb[y as usize * w..(y as usize + 1) * w]
    }

    /// The pixel at `(x, y)` as RGBA, or `None` outside the image.
    #[must_use]
    pub fn get(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = y as usize * self.width as usize + x as usize;
        let a = self.alpha.as_ref().map_or(255, |a| a[i]);
        Some([self.rgb[i * 3], self.rgb[i * 3 + 1], self.rgb[i * 3 + 2], a])
    }

    /// Writes one pixel. Does nothing outside the image. Writing a transparent pixel
    /// into an opaque image creates the alpha plane.
    pub fn set(&mut self, x: u32, y: u32, p: [u8; 4]) {
        if x >= self.width || y >= self.height {
            return;
        }
        let i = y as usize * self.width as usize + x as usize;
        self.rgb[i * 3..i * 3 + 3].copy_from_slice(&p[..3]);
        if p[3] != 255 || self.alpha.is_some() {
            let n = self.rgb.len() / 3;
            self.alpha.get_or_insert_with(|| vec![255; n])[i] = p[3];
        }
    }

    /// Interleaved RGBA copy, for encoders and display.
    #[must_use]
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.rgb.len() / 3 * 4);
        for (i, p) in self.rgb.chunks_exact(3).enumerate() {
            out.extend_from_slice(p);
            out.push(self.alpha.as_ref().map_or(255, |a| a[i]));
        }
        out
    }

    /// Bytes held by the pixel data.
    #[must_use]
    pub fn heap_size(&self) -> usize {
        self.rgb.len() + self.alpha.as_ref().map_or(0, Vec::len)
    }

    /// Every pixel repeated `k` times on both axes, for crisp enlarged output.
    ///
    /// # Errors
    ///
    /// Fails when `k` is 0 or the result would not fit in memory.
    pub fn upscale(&self, k: u32) -> Result<Image> {
        if k == 1 {
            return Ok(self.clone());
        }
        let (w, h) = (self.width.checked_mul(k), self.height.checked_mul(k));
        let (Some(w), Some(h)) = (w, h) else {
            return Err(Error::Invalid {
                name: "scale",
                reason: format!("{k}x is too large"),
            });
        };
        let len = checked_len(w, h, 3)?;
        let k = k as usize;
        let mut rgb = Vec::with_capacity(len);
        let mut alpha = self.alpha.as_ref().map(|_| Vec::with_capacity(len / 3));
        for y in 0..self.height {
            let row = self.row(y);
            let start = rgb.len();
            for p in row.chunks_exact(3) {
                for _ in 0..k {
                    rgb.extend_from_slice(p);
                }
            }
            let line = rgb[start..].to_vec();
            for _ in 1..k {
                rgb.extend_from_slice(&line);
            }
            if let (Some(out), Some(a)) = (&mut alpha, &self.alpha) {
                let src =
                    &a[y as usize * self.width as usize..(y as usize + 1) * self.width as usize];
                for _ in 0..k {
                    for &v in src {
                        out.extend(std::iter::repeat_n(v, k));
                    }
                }
            }
        }
        Image::from_parts(w, h, rgb, alpha)
    }

    /// Converts to the working format.
    #[must_use]
    pub fn to_work(&self) -> Work {
        let mut w = Work::empty();
        self.write_work(&mut w);
        w
    }

    /// Converts to the working format, reusing the allocation of `out`.
    pub fn write_work(&self, out: &mut Work) {
        out.width = self.width;
        out.height = self.height;
        out.px.clear();
        out.px.reserve(self.rgb.len() / 3);
        match &self.alpha {
            Some(a) => out.px.extend(
                self.rgb
                    .chunks_exact(3)
                    .zip(a)
                    .map(|(p, &a)| [to15(p[0]), to15(p[1]), to15(p[2]), to15(a)]),
            ),
            None => out.px.extend(
                self.rgb
                    .chunks_exact(3)
                    .map(|p| [to15(p[0]), to15(p[1]), to15(p[2]), OPAQUE]),
            ),
        }
    }
}

/// An image in the 15-bit working format.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Work {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Pixels, row by row.
    pub px: Vec<Px>,
}

impl Work {
    /// A zero sized image with no allocation, meant to be filled later.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            width: 0,
            height: 0,
            px: Vec::new(),
        }
    }

    /// Creates an image filled with one pixel value.
    ///
    /// # Errors
    ///
    /// Fails when a side is zero or the size does not fit in memory.
    pub fn new(width: u32, height: u32, fill: Px) -> Result<Self> {
        let len = checked_len(width, height, 1)?;
        Ok(Self {
            width,
            height,
            px: vec![fill; len],
        })
    }

    /// Resizes to `width` x `height` and fills with `fill`, keeping the allocation
    /// when it is big enough.
    pub fn reset(&mut self, width: u32, height: u32, fill: Px) {
        self.width = width;
        self.height = height;
        self.px.clear();
        self.px.resize(width as usize * height as usize, fill);
    }

    /// Index of `(x, y)` in [`Work::px`].
    #[inline]
    #[must_use]
    pub fn idx(&self, x: u32, y: u32) -> usize {
        y as usize * self.width as usize + x as usize
    }

    /// The pixel at `(x, y)`. Panics outside the image.
    #[inline]
    #[must_use]
    pub fn at(&self, x: u32, y: u32) -> Px {
        self.px[self.idx(x, y)]
    }

    /// One row. Panics if `y` is out of range.
    #[must_use]
    pub fn row(&self, y: u32) -> &[Px] {
        let w = self.width as usize;
        &self.px[y as usize * w..(y as usize + 1) * w]
    }

    /// Converts back to an 8-bit image. The alpha plane is kept only when some pixel is
    /// not fully opaque.
    ///
    /// # Errors
    ///
    /// Fails when the image is empty.
    pub fn to_image(&self) -> Result<Image> {
        let mut rgb = Vec::with_capacity(self.px.len() * 3);
        let mut opaque = true;
        for p in &self.px {
            rgb.extend([to8(p[0]), to8(p[1]), to8(p[2])]);
            opaque &= to8(p[3]) == 255;
        }
        let alpha = (!opaque).then(|| self.px.iter().map(|p| to8(p[3])).collect());
        Image::from_parts(self.width, self.height, rgb, alpha)
    }
}

/// A palette mapped image: one index per pixel plus a transparency mask.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Indexed {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Palette index of each pixel.
    pub index: Vec<u8>,
    /// `true` for pixels that are fully transparent. Empty when nothing is.
    pub clear: Vec<bool>,
}

impl Indexed {
    /// Creates an opaque image where every pixel uses index `fill`.
    ///
    /// # Errors
    ///
    /// Fails when a side is zero or the size does not fit in memory.
    pub fn new(width: u32, height: u32, fill: u8) -> Result<Self> {
        let len = checked_len(width, height, 1)?;
        Ok(Self {
            width,
            height,
            index: vec![fill; len],
            clear: Vec::new(),
        })
    }

    /// Whether the pixel at linear index `i` is transparent.
    #[inline]
    #[must_use]
    pub fn is_clear(&self, i: usize) -> bool {
        self.clear.get(i).copied().unwrap_or(false)
    }

    /// Marks pixel `i` as transparent or not, creating the mask on first use.
    pub fn set_clear(&mut self, i: usize, clear: bool) {
        if self.clear.is_empty() {
            if !clear {
                return;
            }
            self.clear = vec![false; self.index.len()];
        }
        self.clear[i] = clear;
    }

    /// Expands to an 8-bit image using `colors` (RGB per index). Indices past the end
    /// of `colors` come out black.
    ///
    /// # Errors
    ///
    /// Fails when the image is empty.
    pub fn to_image(&self, colors: &[[u8; 3]]) -> Result<Image> {
        let mut rgb = Vec::with_capacity(self.index.len() * 3);
        for &i in &self.index {
            rgb.extend(colors.get(i as usize).copied().unwrap_or([0; 3]));
        }
        let alpha = (!self.clear.is_empty() && self.clear.iter().any(|&c| c)).then(|| {
            self.clear
                .iter()
                .map(|&c| if c { 0 } else { 255 })
                .collect()
        });
        Image::from_parts(self.width, self.height, rgb, alpha)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_round_trip_is_exact() {
        for v in 0..=255u8 {
            assert_eq!(to8(to15(v)), v);
        }
        assert_eq!(to8(MAX15), 255);
        assert_eq!(to8(u16::MAX), 255);
    }

    #[test]
    fn opaque_rgba_drops_alpha_plane() {
        let img = Image::from_rgba(2, 1, &[1, 2, 3, 255, 4, 5, 6, 255]).unwrap();
        assert!(img.alpha().is_none());
        assert_eq!(img.rgb(), &[1, 2, 3, 4, 5, 6]);
        assert_eq!(img.heap_size(), 6);
    }

    #[test]
    fn rgba_round_trip_keeps_alpha() {
        let data = [1, 2, 3, 0, 4, 5, 6, 128, 7, 8, 9, 255, 10, 11, 12, 1];
        let img = Image::from_rgba(2, 2, &data).unwrap();
        assert_eq!(img.to_rgba(), data);
        assert_eq!(img.get(1, 0), Some([4, 5, 6, 128]));
    }

    #[test]
    fn work_round_trip() {
        let data = [9, 200, 31, 77, 255, 0, 128, 255];
        let img = Image::from_rgba(1, 2, &data).unwrap();
        let back = img.to_work().to_image().unwrap();
        assert_eq!(back, img);
    }

    #[test]
    fn bounds_are_checked() {
        let mut img = Image::new(3, 2, [0, 0, 0]).unwrap();
        assert_eq!(img.get(3, 0), None);
        assert_eq!(img.get(0, 2), None);
        img.set(5, 5, [1, 1, 1, 0]);
        assert!(img.alpha().is_none());
        img.set(2, 1, [1, 2, 3, 0]);
        assert_eq!(img.get(2, 1), Some([1, 2, 3, 0]));
        assert_eq!(img.row(1), &[0, 0, 0, 0, 0, 0, 1, 2, 3]);
    }

    #[test]
    fn bad_sizes_are_rejected() {
        assert!(Image::new(0, 4, [0; 3]).is_err());
        assert!(Image::from_parts(2, 2, vec![0; 11], None).is_err());
        assert!(Image::from_parts(2, 2, vec![0; 12], Some(vec![0; 3])).is_err());
        assert!(Image::from_rgba(1, 1, &[0; 3]).is_err());
    }

    #[test]
    fn write_work_reuses_allocation() {
        let img = Image::new(4, 4, [10, 20, 30]).unwrap();
        let mut w = Work::new(8, 8, [0; 4]).unwrap();
        let cap = w.px.capacity();
        img.write_work(&mut w);
        assert_eq!(w.px.capacity(), cap);
        assert_eq!(w.at(3, 3), [to15(10), to15(20), to15(30), OPAQUE]);
    }

    #[test]
    fn upscale_repeats_pixels() {
        let img = Image::from_rgba(2, 1, &[1, 2, 3, 255, 4, 5, 6, 0]).unwrap();
        let big = img.upscale(3).unwrap();
        assert_eq!((big.width(), big.height()), (6, 3));
        assert_eq!(big.get(2, 2), Some([1, 2, 3, 255]));
        assert_eq!(big.get(3, 0), Some([4, 5, 6, 0]));
        assert!(img.upscale(0).is_err());
        assert_eq!(img.upscale(1).unwrap(), img);
    }

    #[test]
    fn indexed_expands_with_mask() {
        let mut ix = Indexed::new(2, 1, 1).unwrap();
        ix.set_clear(0, true);
        let img = ix.to_image(&[[0; 3], [9, 9, 9]]).unwrap();
        assert_eq!(img.to_rgba(), [9, 9, 9, 0, 9, 9, 9, 255]);
        let solid = Indexed::new(1, 1, 7).unwrap().to_image(&[[1; 3]]).unwrap();
        assert_eq!(solid.rgb(), &[0, 0, 0]);
    }
}
