//! Reading and writing image files.
//!
//! Decoding goes through the `image` crate with only the formats xd-art needs: PNG,
//! JPEG, BMP, TGA and (with the `qoi` feature) QOI. Animated GIF has its own path in
//! [`crate::anim`].
//!
//! Large inputs are guarded by a memory limit. None of the decoders used here can
//! decode at a reduced scale, so [`LoadOptions::max_side`] reduces the image right after
//! decoding by averaging blocks. That keeps every later stage cheap, but the decode
//! itself still needs the full image in memory once.

use std::io::Cursor;
use std::path::Path;

use image::{DynamicImage, ExtendedColorType, ImageFormat, ImageReader, Limits};

use crate::buffer::Image;
use crate::{Error, Result};

/// File formats xd-art can write.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    /// PNG, lossless. The default.
    Png,
    /// Windows bitmap.
    Bmp,
    /// Truevision TGA.
    Tga,
    /// Quite OK Image format.
    #[cfg(feature = "qoi")]
    Qoi,
}

impl Format {
    /// Picks the format from a file extension, case insensitive.
    #[must_use]
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "png" => Some(Self::Png),
            "bmp" => Some(Self::Bmp),
            "tga" => Some(Self::Tga),
            #[cfg(feature = "qoi")]
            "qoi" => Some(Self::Qoi),
            _ => None,
        }
    }

    /// Picks the format from the extension of `path`.
    ///
    /// # Errors
    ///
    /// Fails when the extension is missing or not a format xd-art writes.
    pub fn from_path(path: &Path) -> Result<Self> {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        Self::from_extension(ext).ok_or_else(|| Error::Unsupported(format!("output .{ext}")))
    }

    /// The usual file extension, without the dot.
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Bmp => "bmp",
            Self::Tga => "tga",
            #[cfg(feature = "qoi")]
            Self::Qoi => "qoi",
        }
    }

    fn image_format(self) -> ImageFormat {
        match self {
            Self::Png => ImageFormat::Png,
            Self::Bmp => ImageFormat::Bmp,
            Self::Tga => ImageFormat::Tga,
            #[cfg(feature = "qoi")]
            Self::Qoi => ImageFormat::Qoi,
        }
    }
}

/// Limits applied while loading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadOptions {
    /// Largest number of bytes the decoder may allocate. Files that would need more are
    /// rejected before decoding when the header says so.
    pub max_bytes: u64,
    /// When set, images with a side longer than this are reduced by an integer factor
    /// right after decoding.
    pub max_side: Option<u32>,
}

impl Default for LoadOptions {
    fn default() -> Self {
        Self {
            max_bytes: 1 << 30,
            max_side: None,
        }
    }
}

/// A decoded image and what happened on the way.
#[derive(Clone, Debug)]
pub struct Loaded {
    /// The pixels.
    pub image: Image,
    /// Size stored in the file.
    pub source_size: (u32, u32),
    /// Block size used to reduce the image, 1 when it was not reduced.
    pub reduced_by: u32,
}

/// Loads an image with the default limits.
///
/// ```no_run
/// let img = xd_art_core::io::load(std::path::Path::new("photo.jpg"))?;
/// println!("{} x {}", img.width(), img.height());
/// # Ok::<(), xd_art_core::Error>(())
/// ```
///
/// # Errors
///
/// Fails when the file cannot be read or decoded.
pub fn load(path: &Path) -> Result<Image> {
    load_with(path, &LoadOptions::default()).map(|l| l.image)
}

/// Loads an image with explicit limits.
///
/// # Errors
///
/// Fails when the file cannot be read, is not a supported image, or is over the limit.
pub fn load_with(path: &Path, opts: &LoadOptions) -> Result<Loaded> {
    let bytes = std::fs::read(path).map_err(|source| Error::Io {
        path: path.into(),
        source,
    })?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    decode_named(&bytes, name, opts).map_err(|e| match e {
        Error::Decode { reason, .. } => Error::Decode {
            path: path.into(),
            reason,
        },
        other => other,
    })
}

/// Reads only the header and returns the image size.
///
/// # Errors
///
/// Fails when the file cannot be opened or its format is unknown.
pub fn probe(path: &Path) -> Result<(u32, u32)> {
    let reader = ImageReader::open(path)
        .and_then(ImageReader::with_guessed_format)
        .map_err(|source| Error::Io {
            path: path.into(),
            source,
        })?;
    reader.into_dimensions().map_err(|e| Error::Decode {
        path: path.into(),
        reason: e.to_string(),
    })
}

/// Decodes an image held in memory, guessing the format from its first bytes.
///
/// # Errors
///
/// Fails when the bytes are not a supported image or the image is over the limit.
pub fn decode_with(bytes: &[u8], opts: &LoadOptions) -> Result<Loaded> {
    decode_hinted(bytes, None, opts)
}

/// Like [`decode_with`], but falls back to the extension of `name` when the bytes do not
/// say what they are. TGA files have no signature, so they need this.
///
/// # Errors
///
/// Fails when the bytes are not a supported image or the image is over the limit.
pub fn decode_named(bytes: &[u8], name: &str, opts: &LoadOptions) -> Result<Loaded> {
    let ext = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    decode_hinted(bytes, ImageFormat::from_extension(ext), opts)
}

fn decode_hinted(bytes: &[u8], hint: Option<ImageFormat>, opts: &LoadOptions) -> Result<Loaded> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| decode_err(&e))?;
    if reader.format().is_none()
        && let Some(f) = hint
    {
        reader.set_format(f);
    }
    let mut limits = Limits::default();
    limits.max_alloc = Some(opts.max_bytes);
    reader.limits(limits);
    let img = reader.decode().map_err(|e| decode_err(&e))?;
    let source_size = (img.width(), img.height());
    let image = from_dynamic(img)?;
    let factor = match opts.max_side {
        Some(max) if max > 0 => source_size.0.max(source_size.1).div_ceil(max).max(1),
        _ => 1,
    };
    let image = if factor > 1 {
        reduce(&image, factor)?
    } else {
        image
    };
    Ok(Loaded {
        image,
        source_size,
        reduced_by: factor,
    })
}

fn decode_err(e: &dyn std::fmt::Display) -> Error {
    Error::Decode {
        path: "<memory>".into(),
        reason: e.to_string(),
    }
}

fn from_dynamic(img: DynamicImage) -> Result<Image> {
    let (w, h) = (img.width(), img.height());
    if img.color().has_alpha() {
        Image::from_rgba(w, h, img.into_rgba8().as_raw())
    } else {
        Image::from_parts(w, h, img.into_rgb8().into_raw(), None)
    }
}

/// Averages `factor` x `factor` blocks. Partial blocks at the right and bottom edges
/// average the pixels they have.
///
/// # Errors
///
/// Fails only when `factor` is zero.
pub fn reduce(img: &Image, factor: u32) -> Result<Image> {
    if factor == 0 {
        return Err(Error::Invalid {
            name: "reduce factor",
            reason: "must be at least 1".into(),
        });
    }
    let (w, h) = (img.width(), img.height());
    let (ow, oh) = (w.div_ceil(factor), h.div_ceil(factor));
    let mut rgb = Vec::with_capacity(ow as usize * oh as usize * 3);
    let mut alpha = img
        .alpha()
        .map(|_| Vec::with_capacity(ow as usize * oh as usize));
    for by in 0..oh {
        for bx in 0..ow {
            let mut sum = [0u64; 4];
            let mut n = 0u64;
            for y in by * factor..((by + 1) * factor).min(h) {
                for x in bx * factor..((bx + 1) * factor).min(w) {
                    let p = img.get(x, y).unwrap_or_default();
                    for (s, v) in sum.iter_mut().zip(p) {
                        *s += u64::from(v);
                    }
                    n += 1;
                }
            }
            rgb.extend(sum[..3].iter().map(|s| ((s + n / 2) / n) as u8));
            if let Some(a) = &mut alpha {
                a.push(((sum[3] + n / 2) / n) as u8);
            }
        }
    }
    Image::from_parts(ow, oh, rgb, alpha)
}

/// Encodes to bytes.
///
/// ```
/// use xd_art_core::buffer::Image;
/// use xd_art_core::io::{self, Format, LoadOptions};
///
/// let img = Image::new(4, 2, [10, 20, 30])?;
/// let png = io::encode(&img, Format::Png)?;
/// let back = io::decode_with(&png, &LoadOptions::default())?.image;
/// assert_eq!(back.get(3, 1), Some([10, 20, 30, 255]));
/// # Ok::<(), xd_art_core::Error>(())
/// ```
///
/// # Errors
///
/// Fails when the encoder rejects the image.
pub fn encode(img: &Image, format: Format) -> Result<Vec<u8>> {
    let mut out = Cursor::new(Vec::new());
    let res = match img.alpha() {
        Some(_) => image::write_buffer_with_format(
            &mut out,
            &img.to_rgba(),
            img.width(),
            img.height(),
            ExtendedColorType::Rgba8,
            format.image_format(),
        ),
        None => image::write_buffer_with_format(
            &mut out,
            img.rgb(),
            img.width(),
            img.height(),
            ExtendedColorType::Rgb8,
            format.image_format(),
        ),
    };
    res.map_err(|e| Error::Encode {
        path: "<memory>".into(),
        reason: e.to_string(),
    })?;
    Ok(out.into_inner())
}

/// Saves to `path`, picking the format from the extension.
///
/// # Errors
///
/// Fails on an unknown extension, an encoder error or a write error.
pub fn save(img: &Image, path: &Path) -> Result<()> {
    let format = Format::from_path(path)?;
    let bytes = encode(img, format).map_err(|e| match e {
        Error::Encode { reason, .. } => Error::Encode {
            path: path.into(),
            reason,
        },
        other => other,
    })?;
    std::fs::write(path, bytes).map_err(|source| Error::Io {
        path: path.into(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Image {
        let mut img = Image::new(5, 3, [10, 20, 30]).unwrap();
        img.set(4, 2, [200, 100, 50, 255]);
        img
    }

    #[test]
    fn every_format_round_trips() {
        let img = sample();
        let mut alpha = img.clone();
        alpha.set(0, 0, [1, 2, 3, 40]);
        for f in [
            Format::Png,
            Format::Bmp,
            Format::Tga,
            #[cfg(feature = "qoi")]
            Format::Qoi,
        ] {
            for src in [&img, &alpha] {
                let bytes = encode(src, f).unwrap();
                let name = format!("x.{}", f.extension());
                let back = decode_named(&bytes, &name, &LoadOptions::default()).unwrap();
                assert_eq!(&back.image, src, "{f:?}");
                assert_eq!(back.reduced_by, 1);
            }
        }
    }

    #[test]
    fn reduces_large_images() {
        let img = Image::new(10, 4, [8, 8, 8]).unwrap();
        let bytes = encode(&img, Format::Png).unwrap();
        let l = decode_with(
            &bytes,
            &LoadOptions {
                max_side: Some(4),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(l.reduced_by, 3);
        assert_eq!(l.source_size, (10, 4));
        assert_eq!((l.image.width(), l.image.height()), (4, 2));
        assert_eq!(l.image.get(3, 1), Some([8, 8, 8, 255]));
    }

    #[test]
    fn reduce_averages_blocks() {
        let img = Image::from_rgba(2, 1, &[0, 0, 0, 255, 255, 255, 255, 0]).unwrap();
        let r = reduce(&img, 2).unwrap();
        assert_eq!(r.get(0, 0), Some([128, 128, 128, 128]));
        assert!(reduce(&img, 0).is_err());
    }

    #[test]
    fn memory_limit_rejects() {
        let bytes = encode(&Image::new(64, 64, [0; 3]).unwrap(), Format::Png).unwrap();
        let opts = LoadOptions {
            max_bytes: 100,
            max_side: None,
        };
        assert!(matches!(
            decode_with(&bytes, &opts),
            Err(Error::Decode { .. })
        ));
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(decode_with(b"not an image", &LoadOptions::default()).is_err());
        assert!(Format::from_path(Path::new("a.jpg")).is_err());
        assert_eq!(Format::from_path(Path::new("A.PNG")).unwrap(), Format::Png);
    }

    #[test]
    fn save_and_probe() {
        let dir = std::env::temp_dir().join(format!("xd-art-io-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("probe.png");
        save(&sample(), &path).unwrap();
        assert_eq!(probe(&path).unwrap(), (5, 3));
        assert_eq!(load(&path).unwrap(), sample());
        let missing = load(&dir.join("missing.png")).unwrap_err();
        assert!(missing.to_string().starts_with("could not access"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
