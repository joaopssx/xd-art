//! The config file: a few hundred bytes, read once at startup, written in the
//! background after changes settle.
//!
//! Format, all integers little endian:
//!
//! ```text
//! "XDCF"  version: u8 (1)
//! theme: u8   flags: u8 (bit 0 left panel open, bit 1 right panel open, bit 2 reduced
//!   motion, bit 3 accent from the image)
//! left width: f32   right width: f32
//! recent count: u8, then for each: length: u16, UTF-8 path
//! recipe: length: u32, UTF-8 recipe JSON of the last settings
//! ```
//!
//! A missing file, a short or damaged file, or another version all give the defaults.
//! There is no migration: the file only holds preferences, and losing them costs a
//! couple of clicks.

use std::path::{Path, PathBuf};

const MAGIC: &[u8; 4] = b"XDCF";
const VERSION: u8 = 1;
/// Recent files kept.
pub const RECENTS: usize = 8;

/// Everything the config file holds.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// Theme variant index.
    pub theme: u8,
    /// Left panel shown.
    pub left_open: bool,
    /// Right panel shown.
    pub right_open: bool,
    /// Turn off transitions.
    pub reduced_motion: bool,
    /// Take the accent color from the image's palette.
    pub accent_from_image: bool,
    /// Left panel width.
    pub left_width: f32,
    /// Right panel width.
    pub right_width: f32,
    /// Most recent first.
    pub recents: Vec<PathBuf>,
    /// The last settings, as a recipe.
    pub recipe: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: 0,
            left_open: true,
            right_open: true,
            reduced_motion: false,
            accent_from_image: false,
            left_width: 290.0,
            right_width: 250.0,
            recents: Vec::new(),
            recipe: String::new(),
        }
    }
}

impl Config {
    /// The bytes of the file.
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(64);
        b.extend_from_slice(MAGIC);
        b.push(VERSION);
        b.push(self.theme);
        b.push(
            u8::from(self.left_open)
                | u8::from(self.right_open) << 1
                | u8::from(self.reduced_motion) << 2
                | u8::from(self.accent_from_image) << 3,
        );
        b.extend_from_slice(&self.left_width.to_le_bytes());
        b.extend_from_slice(&self.right_width.to_le_bytes());
        let recents: Vec<String> = self
            .recents
            .iter()
            .take(RECENTS)
            .map(|p| p.to_string_lossy().into_owned())
            .filter(|s| s.len() <= usize::from(u16::MAX))
            .collect();
        b.push(recents.len() as u8);
        for s in &recents {
            b.extend_from_slice(&(s.len() as u16).to_le_bytes());
            b.extend_from_slice(s.as_bytes());
        }
        b.extend_from_slice(&(self.recipe.len() as u32).to_le_bytes());
        b.extend_from_slice(self.recipe.as_bytes());
        b
    }

    /// Reads the bytes of the file. `None` for anything that is not a complete file of
    /// this version.
    pub fn decode(b: &[u8]) -> Option<Self> {
        let mut r = Reader(b);
        if r.take(4)? != MAGIC || r.u8()? != VERSION {
            return None;
        }
        let theme = r.u8()?;
        let flags = r.u8()?;
        let left_width = f32::from_le_bytes(r.take(4)?.try_into().ok()?);
        let right_width = f32::from_le_bytes(r.take(4)?.try_into().ok()?);
        let mut recents = Vec::new();
        for _ in 0..r.u8()? {
            let n = u16::from_le_bytes(r.take(2)?.try_into().ok()?);
            recents.push(PathBuf::from(
                std::str::from_utf8(r.take(usize::from(n))?).ok()?,
            ));
        }
        let n = u32::from_le_bytes(r.take(4)?.try_into().ok()?) as usize;
        let recipe = std::str::from_utf8(r.take(n)?).ok()?.to_string();
        if !r.0.is_empty() || !left_width.is_finite() || !right_width.is_finite() {
            return None;
        }
        Some(Self {
            theme,
            left_open: flags & 1 != 0,
            right_open: flags & 2 != 0,
            reduced_motion: flags & 4 != 0,
            accent_from_image: flags & 8 != 0,
            left_width,
            right_width,
            recents,
            recipe,
        })
    }

    /// Puts `path` at the front of the recent files.
    pub fn remember(&mut self, path: &Path) {
        self.recents.retain(|p| p != path);
        self.recents.insert(0, path.to_path_buf());
        self.recents.truncate(RECENTS);
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Some(a)
    }

    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }
}

/// Where the config file lives: `%APPDATA%\xd-art` on Windows, the macOS application
/// support folder, `$XDG_CONFIG_HOME/xd-art` or `~/.config/xd-art` elsewhere.
pub fn path() -> Option<PathBuf> {
    let dir = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    }?;
    Some(dir.join("xd-art").join("config.bin"))
}

/// Reads the config: one file read, defaults on any problem.
pub fn load() -> Config {
    path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| Config::decode(&b))
        .unwrap_or_default()
}

/// Writes the config. Called from a background thread.
pub fn store(cfg: &Config) -> std::io::Result<()> {
    let Some(p) = path() else { return Ok(()) };
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Write next to it and rename, so a crash mid-write leaves the old file.
    let tmp = p.with_extension("tmp");
    std::fs::write(&tmp, cfg.encode())?;
    std::fs::rename(tmp, p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut c = Config {
            theme: 2,
            left_open: false,
            reduced_motion: true,
            left_width: 333.5,
            ..Config::default()
        };
        c.remember(Path::new("a.png"));
        c.remember(Path::new("b.png"));
        c.remember(Path::new("a.png"));
        c.recipe = "{\"xd-art\":1}".into();
        let b = c.encode();
        assert!(b.len() < 100);
        assert_eq!(Config::decode(&b), Some(c.clone()));
        assert_eq!(
            c.recents,
            vec![PathBuf::from("a.png"), PathBuf::from("b.png")]
        );
    }

    #[test]
    fn damage_gives_none() {
        let b = Config::default().encode();
        assert!(Config::decode(&b[..b.len() - 1]).is_none());
        let mut v = b.clone();
        v[4] = 9;
        assert!(Config::decode(&v).is_none(), "other version");
        assert!(Config::decode(b"").is_none());
        let mut extra = b.clone();
        extra.push(0);
        assert!(Config::decode(&extra).is_none());
    }

    #[test]
    fn recents_are_capped() {
        let mut c = Config::default();
        for i in 0..20 {
            c.remember(Path::new(&format!("{i}.png")));
        }
        assert_eq!(c.recents.len(), RECENTS);
        assert_eq!(c.recents[0], PathBuf::from("19.png"));
    }
}
