//! Icons compiled into the binary.
//!
//! The SVGs in `assets/icons` are drawn by hand for xd-art, 16x16 with a 1.5 pixel
//! stroke, each under 400 bytes. gpui reads them through this source the first time an
//! icon is shown, so none of them are parsed at startup.

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        const ICONS: &[(&str, &[u8])] = &[
            $((concat!("icons/", $name, ".svg"), include_bytes!(concat!("../../../assets/icons/", $name, ".svg")))),*
        ];
    };
}

icons!(
    "open",
    "save",
    "undo",
    "redo",
    "compare",
    "grid",
    "fullscreen",
    "palette",
    "chevron-right",
    "chevron-down",
    "close",
    "lock",
    "history",
    "command",
);

/// The embedded files.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(_, b)| Cow::Borrowed(*b)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .filter(|(p, _)| p.starts_with(path))
            .map(|(p, _)| SharedString::from(*p))
            .collect())
    }
}

/// The path of an icon, for `svg().path(...)`.
pub fn icon(name: &str) -> SharedString {
    format!("icons/{name}.svg").into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_small_and_loads() {
        for (path, bytes) in ICONS {
            assert!(bytes.len() < 1024, "{path} is {} bytes", bytes.len());
            assert!(bytes.starts_with(b"<svg"), "{path}");
            assert!(Assets.load(path).unwrap().is_some());
        }
        assert!(Assets.load("icons/nope.svg").unwrap().is_none());
        assert_eq!(Assets.list("icons/").unwrap().len(), ICONS.len());
    }
}
