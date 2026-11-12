//! Palettes and palette files.
//!
//! Formats, read and written:
//!
//! - `.gpl`, GIMP palette. Lines starting with `#` are comments; every other line that
//!   starts with three numbers is a color. A `Name:` line names the palette.
//! - `.hex`, one `rrggbb` per line, as exported by Lospec and others.
//! - `.pal`, JASC-PAL: three header lines, then `r g b` lines.
//! - `.png` (or any image format the engine reads): the distinct colors of the image,
//!   in reading order, up to 256. The original takes the first 256 pixels as they come,
//!   duplicates included; skipping duplicates gives the same palette for the usual one
//!   pixel per color strips and a usable one for anything else.

use std::fmt::Write as _;
use std::path::Path;

use crate::buffer::Image;
use crate::{Error, Result};

/// Most colors a palette can have. Indices are stored as `u8`.
pub const MAX_COLORS: usize = 256;

/// An ordered list of up to 256 RGB colors.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Palette {
    /// Display name. May be empty.
    pub name: String,
    colors: Vec<[u8; 3]>,
}

fn bad(reason: impl Into<String>) -> Error {
    Error::Invalid {
        name: "palette",
        reason: reason.into(),
    }
}

fn hex6(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    if s.len() < 6 || !s.is_char_boundary(6) {
        return None;
    }
    let v = u32::from_str_radix(&s[..6], 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

fn three_numbers(line: &str) -> Option<[u8; 3]> {
    let mut it = line.split_whitespace().map(str::parse::<u32>);
    let mut c = [0u8; 3];
    for v in &mut c {
        *v = it.next()?.ok()?.min(255) as u8;
    }
    Some(c)
}

impl Palette {
    /// Creates a palette.
    ///
    /// # Errors
    ///
    /// Fails when `colors` is empty or has more than 256 entries.
    pub fn new(name: impl Into<String>, colors: Vec<[u8; 3]>) -> Result<Self> {
        if colors.is_empty() {
            return Err(bad("has no colors"));
        }
        if colors.len() > MAX_COLORS {
            return Err(bad(format!(
                "has {} colors, at most 256 are allowed",
                colors.len()
            )));
        }
        Ok(Self {
            name: name.into(),
            colors,
        })
    }

    /// The colors, in order.
    #[must_use]
    pub fn colors(&self) -> &[[u8; 3]] {
        &self.colors
    }

    /// Mutable access to the colors. The length cannot change through this.
    pub fn colors_mut(&mut self) -> &mut [[u8; 3]] {
        &mut self.colors
    }

    /// Number of colors.
    #[must_use]
    pub fn len(&self) -> usize {
        self.colors.len()
    }

    /// Always false: a palette has at least one color.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.colors.is_empty()
    }

    /// Reads a GIMP palette.
    ///
    /// # Errors
    ///
    /// Fails when no color line is found or there are more than 256.
    pub fn parse_gpl(text: &str) -> Result<Self> {
        let mut name = String::new();
        let mut colors = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if let Some(n) = line.strip_prefix("Name:") {
                name = n.trim().to_string();
            } else if !line.starts_with('#')
                && let Some(c) = three_numbers(line)
            {
                colors.push(c);
            }
        }
        Self::new(name, colors)
    }

    /// Reads a `.hex` file.
    ///
    /// # Errors
    ///
    /// Fails on a line that is not a hex color, or when there are no colors.
    pub fn parse_hex(text: &str) -> Result<Self> {
        let mut colors = Vec::new();
        for (n, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            colors
                .push(hex6(line).ok_or_else(|| bad(format!("line {} is not a hex color", n + 1)))?);
        }
        Self::new("", colors)
    }

    /// Reads a JASC-PAL file.
    ///
    /// # Errors
    ///
    /// Fails when the header is missing or there are no colors.
    pub fn parse_pal(text: &str) -> Result<Self> {
        let mut lines = text.lines();
        if lines.next().map(str::trim) != Some("JASC-PAL") {
            return Err(bad("missing JASC-PAL header"));
        }
        let colors = lines.skip(2).filter_map(three_numbers).collect();
        Self::new("", colors)
    }

    /// The distinct colors of an image in reading order. Transparent pixels are skipped.
    ///
    /// # Errors
    ///
    /// Fails when the image has more than 256 distinct opaque colors or none.
    pub fn from_image(img: &Image) -> Result<Self> {
        let mut colors: Vec<[u8; 3]> = Vec::new();
        for y in 0..img.height() {
            for x in 0..img.width() {
                let p = img.get(x, y).unwrap_or_default();
                let c = [p[0], p[1], p[2]];
                if p[3] >= 128 && !colors.contains(&c) {
                    if colors.len() == MAX_COLORS {
                        return Err(bad("image has more than 256 colors"));
                    }
                    colors.push(c);
                }
            }
        }
        Self::new("", colors)
    }

    /// Loads a palette file, picking the format from the extension. Anything that is not
    /// `.gpl`, `.hex` or `.pal` is read as an image.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be read or parsed.
    pub fn load(path: &Path) -> Result<Self> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let text = || {
            std::fs::read_to_string(path).map_err(|source| Error::Io {
                path: path.into(),
                source,
            })
        };
        let parsed = match ext.as_str() {
            "gpl" => Self::parse_gpl(&text()?),
            "hex" => Self::parse_hex(&text()?),
            "pal" => Self::parse_pal(&text()?),
            _ => Self::from_image(&crate::io::load(path)?),
        };
        let mut p = parsed.map_err(|e| Error::Decode {
            path: path.into(),
            reason: e.to_string(),
        })?;
        if p.name.is_empty() {
            p.name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
        }
        Ok(p)
    }

    /// GIMP palette text.
    #[must_use]
    pub fn to_gpl(&self) -> String {
        let mut s = String::from("GIMP Palette\n");
        if !self.name.is_empty() {
            let _ = writeln!(s, "Name: {}", self.name);
        }
        let _ = writeln!(s, "#Colors: {}", self.colors.len());
        for [r, g, b] in &self.colors {
            let _ = writeln!(s, "{r:3} {g:3} {b:3}\t{r:02x}{g:02x}{b:02x}");
        }
        s
    }

    /// One `rrggbb` per line.
    #[must_use]
    pub fn to_hex(&self) -> String {
        self.colors.iter().fold(String::new(), |mut s, [r, g, b]| {
            let _ = writeln!(s, "{r:02x}{g:02x}{b:02x}");
            s
        })
    }

    /// JASC-PAL text.
    #[must_use]
    pub fn to_pal(&self) -> String {
        let mut s = format!("JASC-PAL\n0100\n{}\n", self.colors.len());
        for [r, g, b] in &self.colors {
            let _ = writeln!(s, "{r} {g} {b}");
        }
        s
    }

    /// A one pixel high strip with every color.
    #[must_use]
    pub fn to_image(&self) -> Image {
        let rgb = self.colors.iter().flatten().copied().collect();
        Image::from_parts(self.colors.len() as u32, 1, rgb, None)
            .unwrap_or_else(|_| unreachable!("palette is never empty"))
    }

    /// Saves to `path`, picking the format from the extension (`.gpl`, `.hex`, `.pal`,
    /// or an image format).
    ///
    /// # Errors
    ///
    /// Fails on a write error or an unknown extension.
    pub fn save(&self, path: &Path) -> Result<()> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let text = match ext.as_str() {
            "gpl" => self.to_gpl(),
            "hex" => self.to_hex(),
            "pal" => self.to_pal(),
            _ => return crate::io::save(&self.to_image(), path),
        };
        std::fs::write(path, text).map_err(|source| Error::Io {
            path: path.into(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Palette {
        Palette::new("test", vec![[0, 0, 0], [255, 128, 7], [18, 52, 86]]).unwrap()
    }

    #[test]
    fn gpl_round_trip() {
        let p = sample();
        let back = Palette::parse_gpl(&p.to_gpl()).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn gpl_from_gimp() {
        let text =
            "GIMP Palette\nName: Tiny\nColumns: 4\n#\n  0   0   0\tBlack\n255 255 255\tWhite\n";
        let p = Palette::parse_gpl(text).unwrap();
        assert_eq!(p.name, "Tiny");
        assert_eq!(p.colors(), &[[0, 0, 0], [255, 255, 255]]);
    }

    #[test]
    fn hex_and_pal_round_trip() {
        let p = sample();
        assert_eq!(
            Palette::parse_hex(&p.to_hex()).unwrap().colors(),
            p.colors()
        );
        assert_eq!(
            Palette::parse_pal(&p.to_pal()).unwrap().colors(),
            p.colors()
        );
        assert_eq!(
            Palette::parse_hex("#FF8007\r\n\n123456").unwrap().colors(),
            &[[255, 128, 7], [0x12, 0x34, 0x56]]
        );
    }

    #[test]
    fn bad_input_is_rejected() {
        assert!(Palette::parse_hex("zzzzzz").is_err());
        assert!(Palette::parse_hex("").is_err());
        assert!(Palette::parse_gpl("GIMP Palette\n# nothing\n").is_err());
        assert!(Palette::parse_pal("0 0 0\n").is_err());
        assert!(Palette::new("", vec![[0; 3]; 257]).is_err());
        let e = Palette::parse_hex("00ff00\nnope").unwrap_err().to_string();
        assert_eq!(e, "invalid palette: line 2 is not a hex color");
    }

    #[test]
    fn image_palette_keeps_order_and_skips_duplicates() {
        let mut img = Image::new(3, 2, [9, 9, 9]).unwrap();
        img.set(1, 0, [1, 2, 3, 255]);
        img.set(2, 1, [4, 5, 6, 0]);
        let p = Palette::from_image(&img).unwrap();
        assert_eq!(p.colors(), &[[9, 9, 9], [1, 2, 3]]);
        assert_eq!(
            Palette::from_image(&p.to_image()).unwrap().colors(),
            p.colors()
        );
    }

    #[test]
    fn files_round_trip() {
        let dir = std::env::temp_dir().join(format!("xd-art-pal-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for ext in ["gpl", "hex", "pal", "png"] {
            let path = dir.join(format!("p.{ext}"));
            sample().save(&path).unwrap();
            let back = Palette::load(&path).unwrap();
            assert_eq!(back.colors(), sample().colors(), "{ext}");
        }
        assert_eq!(Palette::load(&dir.join("p.hex")).unwrap().name, "p");
        assert!(Palette::load(&dir.join("missing.gpl")).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
