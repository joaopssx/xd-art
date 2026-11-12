//! Everything that controls a conversion.
//!
//! [`Settings`] is split in one struct per pipeline stage. The pipeline fingerprints
//! each part on its own, so changing the dither mode does not resample the image.
//! Defaults follow the original program's interface where it has the same option.

use crate::{Error, Result};

/// How the source is reduced to the target size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum SampleMode {
    /// Pick the closest source pixel.
    #[default]
    Nearest,
    /// Linear interpolation of the 4 nearest pixels.
    Bilinear,
    /// Catmull-Rom over 4x4 pixels.
    Bicubic,
    /// Lanczos with 3 lobes, 6x6 pixels.
    Lanczos,
    /// Average of every source pixel that falls in the target pixel.
    Box,
    /// Most common color of the area, found with a small k-means.
    Cluster,
    /// Average of the area weighted by similarity to its center pixel, so edges stay
    /// sharp. Strength in [`Sample::edge_strength`].
    Edge,
}

impl SampleMode {
    /// Every mode, in menu order.
    pub const ALL: [Self; 7] = [
        Self::Nearest,
        Self::Bilinear,
        Self::Bicubic,
        Self::Lanczos,
        Self::Box,
        Self::Cluster,
        Self::Edge,
    ];

    /// Short lowercase name used in the CLI, recipes and Lua.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Nearest => "nearest",
            Self::Bilinear => "bilinear",
            Self::Bicubic => "bicubic",
            Self::Lanczos => "lanczos",
            Self::Box => "box",
            Self::Cluster => "cluster",
            Self::Edge => "edge",
        }
    }

    /// Parses a name produced by [`SampleMode::name`].
    #[must_use]
    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == s)
    }
}

/// Target size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Size {
    /// A fixed size in pixels. With `keep_aspect`, the height follows the width.
    Absolute {
        /// Width in pixels.
        width: u32,
        /// Height in pixels, ignored with `keep_aspect`.
        height: u32,
        /// Derive the height from the source aspect ratio.
        keep_aspect: bool,
    },
    /// Divide the source size by a factor on each axis.
    Relative {
        /// Horizontal divisor.
        x: u32,
        /// Vertical divisor.
        y: u32,
    },
}

impl Default for Size {
    fn default() -> Self {
        Self::Absolute {
            width: 64,
            height: 64,
            keep_aspect: false,
        }
    }
}

impl Size {
    /// The output size for a source of `w` x `h`. Never zero.
    #[must_use]
    pub fn resolve(self, w: u32, h: u32) -> (u32, u32) {
        let (ow, oh) = match self {
            Self::Absolute {
                width,
                keep_aspect: true,
                ..
            } => {
                let oh = (u64::from(width) * u64::from(h) + u64::from(w) / 2) / u64::from(w.max(1));
                (width, oh.min(u64::from(u32::MAX)) as u32)
            }
            Self::Absolute { width, height, .. } => (width, height),
            Self::Relative { x, y } => (w / x.max(1), h / y.max(1)),
        };
        (ow.max(1), oh.max(1))
    }
}

/// Blur applied to the source before sampling.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blur {
    /// Radius in source pixels. 0 turns it off.
    pub radius: f32,
    /// Use the original program's 3-pass box blur instead of a true gaussian.
    pub box_approx: bool,
    /// From this radius up, the gaussian is approximated with three running-sum box
    /// passes, whose cost does not depend on the radius.
    pub box_from: f32,
}

impl Default for Blur {
    fn default() -> Self {
        Self {
            radius: 0.0,
            box_approx: false,
            box_from: 5.0,
        }
    }
}

/// Sampling options.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    /// Interpolation method.
    pub mode: SampleMode,
    /// Target size.
    pub size: Size,
    /// Horizontal sample offset, in output pixels, `-1..=1`.
    pub offset_x: f32,
    /// Vertical sample offset, in output pixels, `-1..=1`.
    pub offset_y: f32,
    /// How strongly [`SampleMode::Edge`] keeps edges, `0..=1`. 0 is a plain box average.
    pub edge_strength: f32,
}

impl Default for Sample {
    fn default() -> Self {
        Self {
            mode: SampleMode::Nearest,
            size: Size::default(),
            offset_x: 0.0,
            offset_y: 0.0,
            edge_strength: 0.6,
        }
    }
}

/// Color adjustments applied to the sampled image.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Adjust {
    /// Unsharp mask amount, 0 turns it off.
    pub sharpen: f32,
    /// Added to every channel, `-1..=1`.
    pub brightness: f32,
    /// 1 leaves contrast alone.
    pub contrast: f32,
    /// 1 leaves saturation alone, 0 is grayscale.
    pub saturation: f32,
    /// Hue rotation in degrees.
    pub hue: f32,
    /// Exponent applied to every channel, 1 leaves it alone.
    pub gamma: f32,
    /// Multiplied into every pixel. White does nothing.
    pub tint: [u8; 3],
    /// How much of the tint is applied, `0..=1`.
    pub tint_strength: f32,
}

impl Default for Adjust {
    fn default() -> Self {
        Self {
            sharpen: 0.0,
            brightness: 0.0,
            contrast: 1.0,
            saturation: 1.0,
            hue: 0.0,
            gamma: 1.0,
            tint: [255; 3],
            tint_strength: 1.0,
        }
    }
}

/// How the distance between two colors is measured.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Metric {
    /// Squared distance in RGB.
    Rgb,
    /// RGB with per channel weights that depend on the red level.
    Weighted,
    /// The "redmean" approximation of perceived difference.
    #[default]
    Redmean,
    /// Euclidean distance in CIELAB.
    Cie76,
    /// CIE94 in CIELAB.
    Cie94,
    /// CIEDE2000 in CIELAB. The most accurate and the slowest.
    Ciede2000,
}

impl Metric {
    /// Every metric, in menu order.
    pub const ALL: [Self; 6] = [
        Self::Rgb,
        Self::Weighted,
        Self::Redmean,
        Self::Cie76,
        Self::Cie94,
        Self::Ciede2000,
    ];

    /// Short lowercase name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Rgb => "rgb",
            Self::Weighted => "weighted",
            Self::Redmean => "redmean",
            Self::Cie76 => "cie76",
            Self::Cie94 => "cie94",
            Self::Ciede2000 => "ciede2000",
        }
    }

    /// Parses a name produced by [`Metric::name`].
    #[must_use]
    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == s)
    }

    /// Whether this metric is cheap enough to use while a slider is moving.
    #[must_use]
    pub fn is_cheap(self) -> bool {
        matches!(self, Self::Rgb | Self::Weighted | Self::Redmean)
    }
}

/// How a palette is built from the image itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Quantizer {
    /// Split the color space in boxes, one color per box.
    MedianCut,
    /// Iterative clustering, seeded with k-means++.
    #[default]
    KMeans,
    /// The most frequent colors, each one discounted by how close it is to the colors
    /// already picked, so small but distinct areas still get a color.
    Balanced,
}

/// Where the palette comes from.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PaletteSource {
    /// A built-in palette by name.
    Builtin(String),
    /// An explicit list of colors.
    Colors(Vec<[u8; 3]>),
    /// Built from the adjusted image.
    Extract {
        /// Number of colors, `1..=256`.
        count: u16,
        /// Algorithm.
        method: Quantizer,
        /// Colors that must be part of the result, whatever the image looks like.
        locked: Vec<[u8; 3]>,
    },
}

impl Default for PaletteSource {
    fn default() -> Self {
        Self::Builtin("pico-8".into())
    }
}

/// Palette options.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct Colors {
    /// Palette to map to.
    pub palette: PaletteSource,
    /// Distance metric.
    pub metric: Metric,
}

/// Dithering method.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum DitherMode {
    /// Closest color, no dithering.
    #[default]
    None,
    /// Ordered, 2x2 Bayer matrix.
    Bayer2,
    /// Ordered, 3x3 Bayer matrix.
    Bayer3,
    /// Ordered, 4x4 Bayer matrix.
    Bayer4,
    /// Ordered, 5x5 Bayer matrix.
    Bayer5,
    /// Ordered, 8x8 Bayer matrix.
    Bayer8,
    /// Ordered, 4x4 clustered dot.
    Cluster4,
    /// Ordered, 8x8 clustered dot.
    Cluster8,
    /// Floyd-Steinberg error diffusion, per channel.
    Floyd,
    /// Floyd-Steinberg with the error averaged over the channels.
    FloydMono,
    /// Stucki error diffusion.
    Stucki,
    /// Burkes error diffusion.
    Burkes,
    /// Sierra error diffusion.
    Sierra,
    /// Two-row Sierra.
    SierraTwoRow,
    /// Sierra lite.
    SierraLite,
    /// Mix the two closest colors in a checker pattern, like picoCAD.
    Picocad,
    /// Assign palette colors to median-cut boxes of the image.
    MedianCut,
}

impl DitherMode {
    /// Every mode, in menu order.
    pub const ALL: [Self; 17] = [
        Self::None,
        Self::Bayer2,
        Self::Bayer3,
        Self::Bayer4,
        Self::Bayer5,
        Self::Bayer8,
        Self::Cluster4,
        Self::Cluster8,
        Self::Floyd,
        Self::FloydMono,
        Self::Stucki,
        Self::Burkes,
        Self::Sierra,
        Self::SierraTwoRow,
        Self::SierraLite,
        Self::Picocad,
        Self::MedianCut,
    ];

    /// Short lowercase name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Bayer2 => "bayer2x2",
            Self::Bayer3 => "bayer3x3",
            Self::Bayer4 => "bayer4x4",
            Self::Bayer5 => "bayer5x5",
            Self::Bayer8 => "bayer8x8",
            Self::Cluster4 => "cluster4x4",
            Self::Cluster8 => "cluster8x8",
            Self::Floyd => "floyd",
            Self::FloydMono => "floyd-mono",
            Self::Stucki => "stucki",
            Self::Burkes => "burkes",
            Self::Sierra => "sierra",
            Self::SierraTwoRow => "sierra-two-row",
            Self::SierraLite => "sierra-lite",
            Self::Picocad => "picocad",
            Self::MedianCut => "median-cut",
        }
    }

    /// Parses a name produced by [`DitherMode::name`].
    #[must_use]
    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == s)
    }

    /// Ordered modes use a threshold matrix and can run on any pixel independently.
    #[must_use]
    pub fn is_ordered(self) -> bool {
        matches!(
            self,
            Self::Bayer2
                | Self::Bayer3
                | Self::Bayer4
                | Self::Bayer5
                | Self::Bayer8
                | Self::Cluster4
                | Self::Cluster8
        )
    }

    /// Error diffusion modes carry error from pixel to pixel.
    #[must_use]
    pub fn is_diffusion(self) -> bool {
        matches!(
            self,
            Self::Floyd
                | Self::FloydMono
                | Self::Stucki
                | Self::Burkes
                | Self::Sierra
                | Self::SierraTwoRow
                | Self::SierraLite
        )
    }
}

/// Dithering options.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dither {
    /// Method.
    pub mode: DitherMode,
    /// Strength of ordered dithering, `0..=1`.
    pub amount: f32,
    /// Pixels with alpha below this become transparent, `0..=255`.
    pub alpha_threshold: u8,
    /// Target color count for the median-cut assignment.
    pub target_colors: u16,
}

impl Default for Dither {
    fn default() -> Self {
        Self {
            mode: DitherMode::None,
            amount: 0.2,
            alpha_threshold: 128,
            target_colors: 8,
        }
    }
}

/// Post-processing on the indexed image.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Post {
    /// Palette index drawn around opaque shapes, on transparent pixels.
    pub outline: Option<u8>,
    /// Palette index drawn on the edge pixels inside opaque shapes.
    pub inline: Option<u8>,
    /// Isolated pixel removal: 0 off, 1 only fully surrounded dots, 2 also dots with
    /// three matching neighbours that are not part of a line.
    pub cleanup: u8,
    /// Palette colors closer than this (8-bit RGB distance) are merged. 0 is off.
    pub merge: u8,
}

/// Final scaling of the result.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Output {
    /// Integer upscale applied when saving, 1 keeps the pixel size.
    pub scale: u32,
}

impl Default for Output {
    fn default() -> Self {
        Self { scale: 1 }
    }
}

/// All settings of one conversion.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Settings {
    /// Blur on the source.
    pub blur: Blur,
    /// Sampling.
    pub sample: Sample,
    /// Color adjustments.
    pub adjust: Adjust,
    /// Palette and metric.
    pub colors: Colors,
    /// Dithering.
    pub dither: Dither,
    /// Post-processing.
    pub post: Post,
    /// Output.
    pub output: Output,
    /// Grayscale map of dithering strength over the image. White is full strength.
    pub mask: Option<Mask>,
}

/// A grayscale map over the image, stretched to whatever size it is applied to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mask {
    width: u32,
    height: u32,
    data: std::sync::Arc<[u8]>,
    id: u64,
}

impl Mask {
    /// Wraps one byte per pixel.
    ///
    /// # Errors
    ///
    /// Fails when the buffer does not match the size.
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Result<Self> {
        if width == 0 || height == 0 || data.len() != width as usize * height as usize {
            return Err(Error::Invalid {
                name: "mask",
                reason: "size does not match the data".into(),
            });
        }
        let id = Fnv::default()
            .u64(u64::from(width))
            .u64(u64::from(height))
            .bytes(&data)
            .finish();
        Ok(Self {
            width,
            height,
            data: data.into(),
            id,
        })
    }

    /// The luminance of an image (Rec. 601 weights), transparent pixels as black.
    ///
    /// # Errors
    ///
    /// Never fails for a valid image; the `Result` is for symmetry with [`Mask::new`].
    pub fn from_image(img: &crate::buffer::Image) -> Result<Self> {
        let mut data = Vec::with_capacity(img.width() as usize * img.height() as usize);
        for y in 0..img.height() {
            for x in 0..img.width() {
                let p = img.get(x, y).unwrap_or_default();
                let l =
                    (299 * u32::from(p[0]) + 587 * u32::from(p[1]) + 114 * u32::from(p[2])) / 1000;
                data.push((l * u32::from(p[3]) / 255) as u8);
            }
        }
        Self::new(img.width(), img.height(), data)
    }

    /// Mask value for pixel `(x, y)` of a `w` x `h` image, by nearest sampling.
    #[inline]
    #[must_use]
    pub fn at(&self, x: u32, y: u32, w: u32, h: u32) -> u8 {
        let mx = (u64::from(x) * u64::from(self.width) / u64::from(w.max(1))) as usize;
        let my = (u64::from(y) * u64::from(self.height) / u64::from(h.max(1))) as usize;
        self.data[my.min(self.height as usize - 1) * self.width as usize
            + mx.min(self.width as usize - 1)]
    }

    /// Size of the mask.
    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The raw values.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// A hash of the contents, part of the map stage's key.
    #[must_use]
    pub fn id(&self) -> u64 {
        self.id
    }
}

fn check(ok: bool, name: &'static str, reason: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(Error::Invalid {
            name,
            reason: reason.into(),
        })
    }
}

fn validate_adjust(a: &Adjust) -> Result<()> {
    for (v, name) in [
        (a.sharpen, "sharpen"),
        (a.brightness, "brightness"),
        (a.contrast, "contrast"),
        (a.saturation, "saturation"),
        (a.hue, "hue"),
        (a.gamma, "gamma"),
        (a.tint_strength, "tint strength"),
    ] {
        check(v.is_finite(), name, "must be a number")?;
    }
    check(
        (0.0..=8.0).contains(&a.sharpen),
        "sharpen",
        "must be in 0..8",
    )?;
    check(
        (-1.0..=1.0).contains(&a.brightness),
        "brightness",
        "must be in -1..1",
    )?;
    check(
        (0.0..=8.0).contains(&a.contrast),
        "contrast",
        "must be in 0..8",
    )?;
    check(
        (0.0..=8.0).contains(&a.saturation),
        "saturation",
        "must be in 0..8",
    )?;
    check((0.0..=8.0).contains(&a.gamma), "gamma", "must be in 0..8")?;
    check(
        (0.0..=1.0).contains(&a.tint_strength),
        "tint strength",
        "must be in 0..1",
    )?;
    Ok(())
}

impl Settings {
    /// Checks every range. The pipeline calls this before running, so a bad value from a
    /// recipe or a script turns into an error instead of a strange image.
    ///
    /// # Errors
    ///
    /// Names the first setting that is out of range.
    pub fn validate(&self) -> Result<()> {
        let finite = f32::is_finite;
        check(
            finite(self.blur.radius) && self.blur.radius >= 0.0,
            "blur",
            "must be >= 0",
        )?;
        check(self.blur.radius <= 512.0, "blur", "must be <= 512")?;
        let s = &self.sample;
        check(
            finite(s.offset_x) && s.offset_x.abs() <= 1.0,
            "offset x",
            "must be in -1..1",
        )?;
        check(
            finite(s.offset_y) && s.offset_y.abs() <= 1.0,
            "offset y",
            "must be in -1..1",
        )?;
        match s.size {
            Size::Absolute {
                width,
                height,
                keep_aspect,
            } => {
                check(
                    width > 0 && (keep_aspect || height > 0),
                    "size",
                    "must be at least 1",
                )?;
                check(
                    width <= 16384 && height <= 16384,
                    "size",
                    "must be at most 16384",
                )?;
            }
            Size::Relative { x, y } => check(x > 0 && y > 0, "scale", "must be at least 1")?,
        }
        validate_adjust(&self.adjust)?;
        match &self.colors.palette {
            PaletteSource::Colors(c) => {
                check(
                    !c.is_empty() && c.len() <= 256,
                    "palette",
                    "needs 1 to 256 colors",
                )?;
            }
            PaletteSource::Extract { count, locked, .. } => {
                check(
                    locked.len() <= usize::from(*count),
                    "palette",
                    "more locked colors than colors",
                )?;
                check(
                    (1..=256).contains(count),
                    "palette",
                    "needs 1 to 256 colors",
                )?;
            }
            PaletteSource::Builtin(name) => check(!name.is_empty(), "palette", "empty name")?,
        }
        let d = &self.dither;
        check(
            finite(d.amount) && (0.0..=1.0).contains(&d.amount),
            "dither amount",
            "must be in 0..1",
        )?;
        check(
            (1..=256).contains(&d.target_colors),
            "target colors",
            "must be in 1..256",
        )?;
        check(self.post.cleanup <= 2, "cleanup", "must be 0, 1 or 2")?;
        check(
            (1..=64).contains(&self.output.scale),
            "output scale",
            "must be in 1..64",
        )?;
        Ok(())
    }
}

/// A stable 64-bit FNV-1a hash. Used for stage fingerprints, so it has to give the same
/// value on every run and every platform, which rules out `DefaultHasher`.
#[derive(Clone, Copy, Debug)]
pub struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    /// Feeds raw bytes.
    pub fn bytes(&mut self, b: &[u8]) -> &mut Self {
        for &x in b {
            self.0 ^= u64::from(x);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
        self
    }

    /// Feeds an integer.
    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.bytes(&v.to_le_bytes())
    }

    /// Feeds a float by its bits. `-0.0` and `0.0` hash the same.
    pub fn f32(&mut self, v: f32) -> &mut Self {
        let v = if v == 0.0 { 0.0 } else { v };
        self.bytes(&v.to_bits().to_le_bytes())
    }

    /// Feeds a string, length first so `"ab","c"` and `"a","bc"` differ.
    pub fn str(&mut self, s: &str) -> &mut Self {
        self.u64(s.len() as u64).bytes(s.as_bytes())
    }

    /// The hash so far.
    #[must_use]
    pub fn finish(&self) -> u64 {
        self.0
    }
}

/// Settings that can be hashed into a stage fingerprint.
pub trait Fingerprint {
    /// Feeds every field that changes the stage output.
    fn feed(&self, h: &mut Fnv);

    /// The fingerprint on its own.
    fn fingerprint(&self) -> u64 {
        let mut h = Fnv::default();
        self.feed(&mut h);
        h.finish()
    }
}

impl Fingerprint for Blur {
    fn feed(&self, h: &mut Fnv) {
        // Radius 0 is off whatever the mode, so both hash the same.
        if self.radius > 0.0 {
            h.f32(self.radius).u64(u64::from(self.box_approx));
            // Only matters when it decides which implementation runs.
            h.u64(u64::from(!self.box_approx && self.radius >= self.box_from));
        } else {
            h.u64(0);
        }
    }
}

impl Fingerprint for Sample {
    fn feed(&self, h: &mut Fnv) {
        if self.mode == SampleMode::Edge {
            h.f32(self.edge_strength);
        }
        h.str(self.mode.name())
            .f32(self.offset_x)
            .f32(self.offset_y);
        match self.size {
            Size::Absolute {
                width,
                height,
                keep_aspect,
            } => {
                h.u64(0).u64(u64::from(width)).u64(u64::from(keep_aspect));
                if !keep_aspect {
                    h.u64(u64::from(height));
                }
            }
            Size::Relative { x, y } => {
                h.u64(1).u64(u64::from(x)).u64(u64::from(y));
            }
        }
    }
}

impl Fingerprint for Adjust {
    fn feed(&self, h: &mut Fnv) {
        h.f32(self.sharpen)
            .f32(self.brightness)
            .f32(self.contrast)
            .f32(self.saturation)
            .f32(self.hue)
            .f32(self.gamma)
            .bytes(&self.tint)
            .f32(self.tint_strength);
    }
}

impl Fingerprint for PaletteSource {
    fn feed(&self, h: &mut Fnv) {
        match self {
            Self::Builtin(name) => {
                h.u64(0).str(name);
            }
            Self::Colors(c) => {
                h.u64(1).u64(c.len() as u64);
                for rgb in c {
                    h.bytes(rgb);
                }
            }
            Self::Extract {
                count,
                method,
                locked,
            } => {
                h.u64(2)
                    .u64(u64::from(*count))
                    .u64(*method as u64)
                    .u64(locked.len() as u64);
                for c in locked {
                    h.bytes(c);
                }
            }
        }
    }
}

impl Fingerprint for Colors {
    fn feed(&self, h: &mut Fnv) {
        self.palette.feed(h);
        h.str(self.metric.name());
    }
}

impl Fingerprint for Dither {
    fn feed(&self, h: &mut Fnv) {
        h.str(self.mode.name())
            .f32(self.amount)
            .u64(u64::from(self.alpha_threshold));
        h.u64(u64::from(self.target_colors));
    }
}

impl Fingerprint for Post {
    fn feed(&self, h: &mut Fnv) {
        h.u64(self.outline.map_or(u64::MAX, u64::from));
        h.u64(self.inline.map_or(u64::MAX, u64::from));
        h.u64(u64::from(self.cleanup)).u64(u64::from(self.merge));
    }
}

impl Fingerprint for Output {
    fn feed(&self, h: &mut Fnv) {
        h.u64(u64::from(self.scale));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_original_interface() {
        let s = Settings::default();
        assert_eq!(s.blur.radius, 0.0);
        assert_eq!(s.sample.mode, SampleMode::Nearest);
        assert_eq!(
            s.sample.size,
            Size::Absolute {
                width: 64,
                height: 64,
                keep_aspect: false
            }
        );
        assert_eq!(
            (s.adjust.contrast, s.adjust.saturation, s.adjust.gamma),
            (1.0, 1.0, 1.0)
        );
        assert_eq!(s.adjust.tint, [255; 3]);
        assert_eq!(s.dither.alpha_threshold, 128);
        assert_eq!(s.dither.amount, 0.2);
        assert_eq!(s.dither.target_colors, 8);
        assert_eq!(s.colors.metric, Metric::Redmean);
        s.validate().unwrap();
    }

    #[test]
    fn names_round_trip() {
        for m in SampleMode::ALL {
            assert_eq!(SampleMode::from_name(m.name()), Some(m));
        }
        for m in Metric::ALL {
            assert_eq!(Metric::from_name(m.name()), Some(m));
        }
        for m in DitherMode::ALL {
            assert_eq!(DitherMode::from_name(m.name()), Some(m));
            assert!(!(m.is_ordered() && m.is_diffusion()));
        }
        assert_eq!(DitherMode::from_name("nope"), None);
    }

    #[test]
    fn sizes_resolve() {
        let abs = Size::Absolute {
            width: 32,
            height: 10,
            keep_aspect: false,
        };
        assert_eq!(abs.resolve(640, 480), (32, 10));
        let keep = Size::Absolute {
            width: 32,
            height: 10,
            keep_aspect: true,
        };
        assert_eq!(keep.resolve(640, 480), (32, 24));
        assert_eq!(keep.resolve(1000, 1), (32, 1));
        assert_eq!(Size::Relative { x: 4, y: 3 }.resolve(640, 480), (160, 160));
        assert_eq!(Size::Relative { x: 8, y: 8 }.resolve(3, 3), (1, 1));
    }

    #[test]
    fn validation_names_the_bad_field() {
        let mut s = Settings::default();
        s.adjust.gamma = f32::NAN;
        let e = s.validate().unwrap_err().to_string();
        assert_eq!(e, "invalid gamma: must be a number");
        let mut s = Settings::default();
        s.colors.palette = PaletteSource::Colors(vec![]);
        assert!(s.validate().is_err());
        let mut s = Settings::default();
        s.sample.size = Size::Relative { x: 0, y: 1 };
        assert!(s.validate().is_err());
    }

    #[test]
    fn fingerprints_track_changes() {
        let a = Settings::default();
        let mut b = a.clone();
        assert_eq!(a.sample.fingerprint(), b.sample.fingerprint());
        b.sample.offset_x = 0.5;
        assert_ne!(a.sample.fingerprint(), b.sample.fingerprint());
        let mut c = a.clone();
        c.adjust.brightness = -0.0;
        assert_eq!(a.adjust.fingerprint(), c.adjust.fingerprint());
        let mut d = a.clone();
        d.blur.box_approx = true;
        assert_eq!(a.blur.fingerprint(), d.blur.fingerprint());
        d.blur.radius = 2.0;
        assert_ne!(a.blur.fingerprint(), d.blur.fingerprint());
    }

    #[test]
    fn fnv_is_stable() {
        // Reference values of 64-bit FNV-1a.
        assert_eq!(Fnv::default().finish(), 0xcbf2_9ce4_8422_2325);
        assert_eq!(Fnv::default().bytes(b"a").finish(), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(
            Fnv::default().bytes(b"foobar").finish(),
            0x8594_4171_f739_67e8
        );
    }
}
