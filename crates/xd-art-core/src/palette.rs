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
//!
//! Built-in palettes are stored as short strings and only turned into a [`Palette`]
//! when one is picked, so having them costs nothing at startup.

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
    locked: Vec<bool>,
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

fn rgb_to_hsv(c: [u8; 3]) -> (f32, f32, f32) {
    let [r, g, b] = c.map(|v| f32::from(v) / 255.0);
    let max = r.max(g).max(b);
    let d = max - r.min(g).min(b);
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (h, if max == 0.0 { 0.0 } else { d / max }, max)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [u8; 3] {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [r, g, b].map(|v| ((v + m) * 255.0).round() as u8)
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
        let locked = vec![false; colors.len()];
        Ok(Self {
            name: name.into(),
            colors,
            locked,
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

    /// Whether color `i` is locked. Locked colors are kept when the palette is rebuilt
    /// from the image.
    #[must_use]
    pub fn is_locked(&self, i: usize) -> bool {
        self.locked.get(i).copied().unwrap_or(false)
    }

    /// Locks or unlocks color `i`. Out of range does nothing.
    pub fn set_locked(&mut self, i: usize, on: bool) {
        if let Some(l) = self.locked.get_mut(i) {
            *l = on;
        }
    }

    /// The locked colors, in order.
    #[must_use]
    pub fn locked_colors(&self) -> Vec<[u8; 3]> {
        self.colors
            .iter()
            .zip(&self.locked)
            .filter(|(_, l)| **l)
            .map(|(c, _)| *c)
            .collect()
    }

    /// Replaces color `i`. Out of range does nothing.
    pub fn set(&mut self, i: usize, c: [u8; 3]) {
        if let Some(v) = self.colors.get_mut(i) {
            *v = c;
        }
    }

    /// Adds a color at the end.
    ///
    /// # Errors
    ///
    /// Fails when the palette already has 256 colors.
    pub fn push(&mut self, c: [u8; 3]) -> Result<()> {
        if self.colors.len() >= MAX_COLORS {
            return Err(bad("already has 256 colors"));
        }
        self.colors.push(c);
        self.locked.push(false);
        Ok(())
    }

    /// Removes color `i`.
    ///
    /// # Errors
    ///
    /// Fails when `i` is out of range or it is the last color.
    pub fn remove(&mut self, i: usize) -> Result<[u8; 3]> {
        if i >= self.colors.len() {
            return Err(bad(format!("has no color {i}")));
        }
        if self.colors.len() == 1 {
            return Err(bad("needs at least one color"));
        }
        self.locked.remove(i);
        Ok(self.colors.remove(i))
    }

    /// Moves color `from` to position `to`, shifting the ones in between. Lock flags
    /// move with their colors.
    pub fn move_to(&mut self, from: usize, to: usize) {
        if from >= self.colors.len() || to >= self.colors.len() || from == to {
            return;
        }
        let c = self.colors.remove(from);
        let l = self.locked.remove(from);
        self.colors.insert(to, c);
        self.locked.insert(to, l);
    }

    fn sort_by_key<K: Ord>(&mut self, key: impl Fn([u8; 3]) -> K) {
        let mut both: Vec<([u8; 3], bool)> = self
            .colors
            .iter()
            .copied()
            .zip(self.locked.iter().copied())
            .collect();
        both.sort_by_key(|(c, _)| key(*c));
        (self.colors, self.locked) = both.into_iter().unzip();
    }

    /// Sorts dark to light (Rec. 601 luma).
    pub fn sort_by_luminance(&mut self) {
        self.sort_by_key(|c| 299 * u32::from(c[0]) + 587 * u32::from(c[1]) + 114 * u32::from(c[2]));
    }

    /// Sorts by hue, grays first (by lightness), then the colors around the wheel.
    pub fn sort_by_hue(&mut self) {
        self.sort_by_key(|c| {
            let (h, s, v) = rgb_to_hsv(c);
            if s < 0.08 {
                (0, 0, (v * 1000.0) as u32)
            } else {
                (1, (h * 10.0) as u32, (v * 1000.0) as u32)
            }
        });
    }

    /// Shifts every unlocked color: hue in degrees, saturation as a factor, value as an
    /// offset in `-1..=1`.
    pub fn adjust_all(&mut self, hue: f32, saturation: f32, value: f32) {
        for (c, l) in self.colors.iter_mut().zip(&self.locked) {
            if *l {
                continue;
            }
            let (h, s, v) = rgb_to_hsv(*c);
            *c = hsv_to_rgb(
                (h + hue).rem_euclid(360.0),
                (s * saturation).clamp(0.0, 1.0),
                (v + value).clamp(0.0, 1.0),
            );
        }
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

/// A palette that ships with xd-art.
#[derive(Clone, Copy, Debug)]
pub struct Builtin {
    /// Name used in settings, recipes, the CLI and Lua.
    pub name: &'static str,
    /// Where the colors come from, one line.
    pub source: &'static str,
    data: Data,
}

#[derive(Clone, Copy, Debug)]
enum Data {
    /// `rrggbb` values separated by spaces.
    Hex(&'static str),
    /// Evenly spaced gray levels.
    Gray(u16),
    /// Every combination of the given levels per channel.
    Cube(&'static [u8], &'static [u8], &'static [u8]),
}

/// Built-in palettes, in menu order. Only names and short strings live here; a palette
/// is built when someone asks for it with [`builtin`].
pub const BUILTINS: &[Builtin] = &[
    Builtin {
        name: "pico-8",
        source: "PICO-8 fantasy console, the 16 colors of its published specification",
        data: Data::Hex(
            "000000 1d2b53 7e2553 008751 ab5236 5f574f c2c3c7 fff1e8 \
             ff004d ffa300 ffec27 00e436 29adff 83769c ff77a8 ffccaa",
        ),
    },
    Builtin {
        name: "game-boy",
        source: "the four greens commonly used to emulate the original Game Boy screen",
        data: Data::Hex("0f380f 306230 8bac0f 9bbc0f"),
    },
    Builtin {
        name: "cga",
        source: "IBM CGA 16 color RGBI palette, with dark yellow shown as brown as on the 5153 monitor",
        data: Data::Hex(
            "000000 0000aa 00aa00 00aaaa aa0000 aa00aa aa5500 aaaaaa \
             555555 5555ff 55ff55 55ffff ff5555 ff55ff ffff55 ffffff",
        ),
    },
    Builtin {
        name: "1-bit",
        source: "black and white",
        data: Data::Hex("000000 ffffff"),
    },
    Builtin {
        name: "gray-4",
        source: "4 evenly spaced grays",
        data: Data::Gray(4),
    },
    Builtin {
        name: "gray-16",
        source: "16 evenly spaced grays",
        data: Data::Gray(16),
    },
    Builtin {
        name: "dungeon-16",
        source: "xd-art, made for this project: browns, reds and stone grays for dark shooter style art",
        data: Data::Hex(
            "0b0907 1f1610 3a2a1c 5b4128 7d5a37 a37a4b c9a46c 3b0e0e \
             6e1a14 a8321e e0642a f2b04a 2b2d2f 4f5357 80868c b8bec4",
        ),
    },
    Builtin {
        name: "console-32",
        source: "xd-art, made for this project: a 32 color set in the spirit of 8-bit home consoles",
        data: Data::Hex(
            "000000 1c1c1c 4a4a4a 7c7c7c bcbcbc f8f8f8 0a1a6e 2448c8 \
             5c94fc a4c8fc 3c0c74 7c3cd8 b884fc 6c0c2c b82c4c f87c9c \
             8c1800 e44010 fc8c4c fcc49c 5c3c00 a87000 f8b800 fce4a0 \
             004400 108c18 58d854 b8f8a8 004c58 00a0a8 58f8e8 d8f8f0",
        ),
    },
    Builtin {
        name: "blocks-24",
        source: "xd-art, made for this project: stone, dirt, wood, leaves, water and ore tones for block game textures",
        data: Data::Hex(
            "1a1a1c 3b3b40 5e5e66 8a8a92 b5b5bc 4a3121 6e4a2f 93683f              b88c5a 2f4a1c 4a7028 6f9c3a 9cc95a 1e3a5c 2f5f8f 4f8fc4              8fc4e8 7a1f1f b83a2a e0b040 f2e6b0 3fc0b0 9a6ac0 e8e8e8",
        ),
    },
    Builtin {
        name: "web-216",
        source: "the 216 color web safe cube, 6 levels per channel",
        data: Data::Cube(
            &[0, 51, 102, 153, 204, 255],
            &[0, 51, 102, 153, 204, 255],
            &[0, 51, 102, 153, 204, 255],
        ),
    },
    Builtin {
        name: "rgb-332",
        source: "8-bit truecolor: 8 levels of red and green, 4 of blue",
        data: Data::Cube(
            &[0, 36, 73, 109, 146, 182, 219, 255],
            &[0, 36, 73, 109, 146, 182, 219, 255],
            &[0, 85, 170, 255],
        ),
    },
];

/// Names of the built-in palettes, in menu order.
pub fn builtin_names() -> impl Iterator<Item = &'static str> {
    BUILTINS.iter().map(|b| b.name)
}

/// Builds a built-in palette by name.
#[must_use]
pub fn builtin(name: &str) -> Option<Palette> {
    let b = BUILTINS.iter().find(|b| b.name == name)?;
    let colors = match b.data {
        Data::Hex(s) => s.split_whitespace().filter_map(hex6).collect(),
        Data::Gray(n) => (0..n)
            .map(|i| [(u32::from(i) * 255 / u32::from(n - 1)) as u8; 3])
            .collect(),
        Data::Cube(r, g, bl) => {
            let mut v = Vec::with_capacity(r.len() * g.len() * bl.len());
            for &r in r {
                for &g in g {
                    for &b in bl {
                        v.push([r, g, b]);
                    }
                }
            }
            v
        }
    };
    Palette::new(b.name, colors).ok()
}

/// Palettes from [Lospec](https://lospec.com/palette-list), behind the
/// `online-palettes` feature.
///
/// Nothing here runs unless called: no request at startup, no background refresh. A
/// palette is fetched once from `https://lospec.com/palette-list/<slug>.json` (the
/// endpoint Lospec's own download links use; checked on 2026-11-12 to return
/// `{"name", "author", "colors"}`) and then read from the cache directory. The colors of
/// a Lospec palette belong to its author, whose name comes with the palette.
#[cfg(feature = "online-palettes")]
pub mod lospec {
    use std::path::Path;
    use std::time::Duration;

    use super::Palette;
    use crate::{Error, Result};

    /// The slug of a palette from a name, a slug or a Lospec link
    /// (`"Pico 8"`, `"pico-8"` and `"https://lospec.com/palette-list/pico-8"` all give
    /// `pico-8`). `None` when nothing usable is left.
    #[must_use]
    pub fn slug(input: &str) -> Option<String> {
        let s = input.trim().trim_end_matches('/');
        let s = s.rsplit('/').next().unwrap_or(s);
        let s = s.strip_suffix(".json").unwrap_or(s);
        let mut out = String::new();
        for ch in s.chars() {
            if ch.is_ascii_alphanumeric() {
                out.push(ch.to_ascii_lowercase());
            } else if (ch == '-' || ch == ' ' || ch == '_')
                && !out.ends_with('-')
                && !out.is_empty()
            {
                out.push('-');
            }
        }
        let out = out.trim_end_matches('-').to_string();
        (!out.is_empty()).then_some(out)
    }

    /// Reads Lospec's JSON. The palette is named `name by author` when there is an
    /// author.
    ///
    /// # Errors
    ///
    /// Fails when the JSON does not have a `colors` list of hex strings.
    pub fn parse(json: &str) -> Result<Palette> {
        let bad = |r: &str| Error::Invalid {
            name: "lospec palette",
            reason: r.into(),
        };
        let v: serde_json::Value = serde_json::from_str(json).map_err(|e| bad(&e.to_string()))?;
        let colors = v["colors"]
            .as_array()
            .ok_or_else(|| bad("no colors list"))?;
        let mut hex = String::new();
        for c in colors {
            hex.push_str(c.as_str().ok_or_else(|| bad("colors are not strings"))?);
            hex.push('\n');
        }
        let mut p = Palette::parse_hex(&hex)?;
        let name = v["name"].as_str().unwrap_or("");
        let author = v["author"].as_str().unwrap_or("");
        p.name = if author.is_empty() {
            name.to_string()
        } else {
            format!("{name} by {author}")
        };
        Ok(p)
    }

    /// Gets a palette, from `cache` when it was fetched before.
    ///
    /// # Errors
    ///
    /// Fails when the input has no usable slug, Lospec cannot be reached, it has no such
    /// palette, or its answer cannot be read.
    pub fn fetch(input: &str, cache: Option<&Path>) -> Result<Palette> {
        let slug = slug(input).ok_or_else(|| Error::Invalid {
            name: "lospec palette",
            reason: format!("{input:?} is not a palette name or link"),
        })?;
        let cached = cache.map(|d| d.join(format!("{slug}.json")));
        if let Some(path) = &cached
            && let Ok(text) = std::fs::read_to_string(path)
            && let Ok(p) = parse(&text)
        {
            return Ok(p);
        }
        let url = format!("https://lospec.com/palette-list/{slug}.json");
        let fail = |e: ureq::Error| match e {
            ureq::Error::StatusCode(404) => Error::Invalid {
                name: "lospec palette",
                reason: format!("lospec has no palette named {slug}"),
            },
            other => Error::Unsupported(format!("could not reach lospec: {other}")),
        };
        let mut resp = ureq::get(&url)
            .header("User-Agent", concat!("xd-art/", env!("CARGO_PKG_VERSION")))
            .config()
            .timeout_global(Some(Duration::from_secs(15)))
            .build()
            .call()
            .map_err(fail)?;
        let text = resp.body_mut().read_to_string().map_err(fail)?;
        let p = parse(&text)?;
        if let Some(path) = &cached {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).ok();
            }
            // A failed cache write only means the next call asks again.
            std::fs::write(path, &text).ok();
        }
        Ok(p)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn slugs_from_names_and_links() {
            assert_eq!(slug("pico-8").as_deref(), Some("pico-8"));
            assert_eq!(slug("Pico 8").as_deref(), Some("pico-8"));
            assert_eq!(
                slug("https://lospec.com/palette-list/sweetie-16").as_deref(),
                Some("sweetie-16")
            );
            assert_eq!(
                slug("https://lospec.com/palette-list/sweetie-16.json").as_deref(),
                Some("sweetie-16")
            );
            assert_eq!(slug("  ").as_deref(), None);
        }

        #[test]
        fn parses_the_lospec_format() {
            let p = parse(r#"{"name":"Tiny","author":"Someone","colors":["000000","FFFFFF"]}"#)
                .unwrap();
            assert_eq!(p.name, "Tiny by Someone");
            assert_eq!(p.colors(), &[[0; 3], [255; 3]]);
            assert!(parse(r#"{"name":"x"}"#).is_err());
            assert!(parse(r#"{"colors":[1,2]}"#).is_err());
        }

        #[test]
        #[ignore = "talks to lospec.com"]
        fn live_fetch() {
            let p = fetch("pico-8", None).unwrap();
            assert_eq!(p.len(), 16);
            assert!(
                fetch("surely-not-a-real-palette-xd", None)
                    .unwrap_err()
                    .to_string()
                    .contains("no palette named")
            );
        }

        #[test]
        fn cache_is_used_without_network() {
            let dir = std::env::temp_dir().join(format!("xd-art-lospec-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("cached-one.json"),
                r#"{"name":"C","author":"","colors":["123456"]}"#,
            )
            .unwrap();
            let p = fetch("Cached One", Some(&dir)).unwrap();
            assert_eq!(p.colors(), &[[0x12, 0x34, 0x56]]);
            std::fs::remove_dir_all(&dir).ok();
        }
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
    fn every_builtin_builds() {
        let expect = [16, 4, 16, 2, 4, 16, 16, 32, 24, 216, 256];
        for (b, n) in BUILTINS.iter().zip(expect) {
            let p = builtin(b.name).unwrap();
            assert_eq!(p.len(), n, "{}", b.name);
            assert!(!b.source.is_empty());
        }
        assert_eq!(builtin_names().count(), expect.len());
        assert!(builtin("nope").is_none());
    }

    #[test]
    fn builtin_values_match_their_sources() {
        let pico = builtin("pico-8").unwrap();
        assert_eq!(pico.colors()[8], [0xff, 0x00, 0x4d]);
        assert_eq!(pico.colors()[15], [0xff, 0xcc, 0xaa]);
        let gray = builtin("gray-4").unwrap();
        assert_eq!(gray.colors(), &[[0; 3], [85; 3], [170; 3], [255; 3]]);
        let cga = builtin("cga").unwrap();
        assert_eq!(cga.colors()[6], [0xaa, 0x55, 0x00]);
        let rgb = builtin("rgb-332").unwrap();
        assert_eq!(rgb.colors()[255], [255, 255, 255]);
    }

    #[test]
    fn editing_keeps_locks_with_their_colors() {
        let mut p = sample();
        p.set_locked(1, true);
        p.move_to(1, 0);
        assert_eq!(p.colors()[0], [255, 128, 7]);
        assert!(p.is_locked(0) && !p.is_locked(1));
        p.sort_by_luminance();
        assert_eq!(p.colors()[0], [0, 0, 0]);
        assert_eq!(p.locked_colors(), vec![[255, 128, 7]]);
        p.push([9, 9, 9]).unwrap();
        assert_eq!(p.remove(3).unwrap(), [9, 9, 9]);
        let mut one = Palette::new("", vec![[0; 3]]).unwrap();
        assert!(one.remove(0).is_err());
        assert!(one.remove(5).is_err());
    }

    #[test]
    fn adjust_all_skips_locked_and_round_trips() {
        let mut p = Palette::new("", vec![[200, 40, 40], [40, 200, 40]]).unwrap();
        p.set_locked(1, true);
        p.adjust_all(0.0, 1.0, 0.0);
        assert_eq!(p.colors(), &[[200, 40, 40], [40, 200, 40]]);
        p.adjust_all(120.0, 1.0, 0.0);
        assert_eq!(p.colors()[0], [40, 200, 40]);
        assert_eq!(p.colors()[1], [40, 200, 40]);
    }

    #[test]
    fn hue_sort_puts_grays_first() {
        let mut p = Palette::new(
            "",
            vec![[0, 0, 255], [128, 128, 128], [255, 0, 0], [0, 255, 0]],
        )
        .unwrap();
        p.sort_by_hue();
        assert_eq!(
            p.colors(),
            &[[128, 128, 128], [255, 0, 0], [0, 255, 0], [0, 0, 255]]
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
