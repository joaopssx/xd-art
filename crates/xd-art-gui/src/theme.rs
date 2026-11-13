//! Every color, size and font of the interface.
//!
//! All four themes are dark. `Dark` uses the base tokens; the others shift them: `Darker`
//! toward black, `Midnight` toward blue, `Plum` toward purple. The accent and the state
//! colors stay recognizable in all of them.

use std::sync::Arc;

use gpui::{App, Font, FontFeatures, FontWeight, Global, Hsla, Pixels, Rgba, px, rgb};

/// One theme's colors.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// Window background, behind the image.
    pub base: Rgba,
    /// Side panels and bars.
    pub panel: Rgba,
    /// Controls and popovers on top of panels.
    pub elevated: Rgba,
    /// Lines between areas and around controls.
    pub border: Rgba,
    /// Main text.
    pub text: Rgba,
    /// Labels, hints, the footer.
    pub muted: Rgba,
    /// Selection, focus, active controls, links on hover.
    pub accent: Rgba,
    /// Errors.
    pub danger: Rgba,
    /// Finished, saved.
    pub success: Rgba,
}

/// The theme variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Variant {
    /// The base tokens.
    #[default]
    Dark,
    /// Closer to black, for dim rooms.
    Darker,
    /// Blue tinted.
    Midnight,
    /// Purple tinted.
    Plum,
}

impl Variant {
    /// Every variant, in menu order.
    pub const ALL: [Self; 4] = [Self::Dark, Self::Darker, Self::Midnight, Self::Plum];

    /// Name for menus and the config file.
    pub fn name(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Darker => "darker",
            Self::Midnight => "midnight",
            Self::Plum => "plum",
        }
    }

    /// The variant's index, stored in the config file.
    pub fn index(self) -> u8 {
        Self::ALL.iter().position(|v| *v == self).unwrap_or(0) as u8
    }

    /// The variant at `i`, falling back to the default.
    pub fn from_index(i: u8) -> Self {
        Self::ALL.get(usize::from(i)).copied().unwrap_or_default()
    }

    /// The colors.
    pub fn palette(self) -> Palette {
        let shared = |base, panel, elevated, border| Palette {
            base: rgb(base),
            panel: rgb(panel),
            elevated: rgb(elevated),
            border: rgb(border),
            text: rgb(0x00df_e2ec),
            muted: rgb(0x008b_91a5),
            accent: rgb(0x008b_7cf6),
            danger: rgb(0x00e5_626f),
            success: rgb(0x005e_c28f),
        };
        match self {
            Self::Dark => shared(0x000e_0f14, 0x0014_161d, 0x001a_1d26, 0x0026_2a36),
            Self::Darker => shared(0x0007_0809, 0x000c_0d11, 0x0012_141a, 0x001c_1f28),
            Self::Midnight => shared(0x000a_0f1c, 0x000f_1526, 0x0015_1d33, 0x0021_2c4a),
            Self::Plum => shared(0x0011_0d17, 0x0018_1220, 0x0020_182b, 0x0030_2540),
        }
    }
}

/// Text sizes. Four, and nothing in between.
pub mod size {
    use super::{Pixels, px};
    /// Footer, status bar, hints.
    pub fn small() -> Pixels {
        px(11.0)
    }
    /// Labels and values.
    pub fn body() -> Pixels {
        px(12.5)
    }
    /// Panel titles.
    pub fn title() -> Pixels {
        px(13.5)
    }
    /// The empty state.
    pub fn large() -> Pixels {
        px(17.0)
    }
}

/// The interface font: the platform's own UI font. See `docs/decisions.md` for why no
/// font is embedded.
pub fn ui_font() -> Font {
    gpui::font(UI_FAMILY)
}

/// The UI font with tabular figures, so numbers in fields do not jump around as they
/// change.
pub fn number_font() -> Font {
    Font {
        features: FontFeatures(Arc::new(vec![("tnum".into(), 1)])),
        weight: FontWeight::NORMAL,
        ..gpui::font(UI_FAMILY)
    }
}

#[cfg(target_os = "windows")]
const UI_FAMILY: &str = "Segoe UI";
#[cfg(target_os = "macos")]
const UI_FAMILY: &str = ".SystemUIFont";
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const UI_FAMILY: &str = "Noto Sans";

/// The current theme, as a gpui global.
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    /// Which variant.
    pub variant: Variant,
    /// Its colors.
    pub c: Palette,
}

impl Global for Theme {}

impl Theme {
    /// Installs `variant` as the current theme.
    pub fn set(variant: Variant, cx: &mut App) {
        cx.set_global(Theme {
            variant,
            c: variant.palette(),
        });
    }

    /// The current theme.
    pub fn get(cx: &App) -> Theme {
        cx.try_global::<Theme>().copied().unwrap_or(Theme {
            variant: Variant::Dark,
            c: Variant::Dark.palette(),
        })
    }
}

/// `color` with alpha `a`, for hover and pressed states.
pub fn alpha(color: Rgba, a: f32) -> Hsla {
    let mut h: Hsla = color.into();
    h.a = a;
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(c: Rgba) -> f32 {
        let lin = |v: f32| {
            if v <= 0.040_45 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
    }

    fn contrast(a: Rgba, b: Rgba) -> f32 {
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn text_meets_wcag_aa_on_every_surface() {
        for v in Variant::ALL {
            let p = v.palette();
            for bg in [p.base, p.panel, p.elevated] {
                assert!(
                    contrast(p.text, bg) >= 4.5,
                    "{v:?} text {}",
                    contrast(p.text, bg)
                );
                assert!(
                    contrast(p.muted, bg) >= 4.5,
                    "{v:?} muted {}",
                    contrast(p.muted, bg)
                );
                assert!(
                    contrast(p.accent, bg) >= 4.5,
                    "{v:?} accent {}",
                    contrast(p.accent, bg)
                );
                assert!(
                    contrast(p.danger, bg) >= 4.5,
                    "{v:?} danger {}",
                    contrast(p.danger, bg)
                );
            }
        }
    }

    #[test]
    fn variants_round_trip_by_index() {
        for v in Variant::ALL {
            assert_eq!(Variant::from_index(v.index()), v);
        }
        assert_eq!(Variant::from_index(200), Variant::Dark);
    }
}
