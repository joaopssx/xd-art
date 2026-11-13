//! Style presets: starting points named after a look.
//!
//! A preset is only a [`Settings`] value. Palettes come from [`crate::palette`], where
//! the source of each one is written down; no preset claims to reproduce any console or
//! game exactly.

use crate::settings::{DitherMode, Metric, PaletteSource, SampleMode, Settings, Size};

/// A named preset.
#[derive(Clone, Copy, Debug)]
pub struct Preset {
    /// Name used in the interface, the CLI and Lua.
    pub name: &'static str,
    /// One line on what it does.
    pub about: &'static str,
    build: fn(&mut Settings),
}

impl Preset {
    /// The settings of this preset.
    #[must_use]
    pub fn settings(&self) -> Settings {
        let mut s = Settings::default();
        (self.build)(&mut s);
        s
    }
}

fn palette(s: &mut Settings, name: &str) {
    s.colors.palette = PaletteSource::Builtin(name.into());
}

/// Every built-in preset, in menu order.
pub const PRESETS: &[Preset] = &[
    Preset {
        name: "game-boy",
        about: "160 pixels wide, four greens, ordered dithering",
        build: |s| {
            palette(s, "game-boy");
            s.sample.size = Size::Absolute {
                width: 160,
                height: 144,
                keep_aspect: true,
            };
            s.sample.mode = SampleMode::Box;
            s.dither.mode = DitherMode::Bayer4;
            s.dither.amount = 0.35;
            s.adjust.contrast = 1.15;
        },
    },
    Preset {
        name: "pico-8",
        about: "128x128 with the PICO-8 palette and light Floyd-Steinberg",
        build: |s| {
            palette(s, "pico-8");
            s.sample.size = Size::Absolute {
                width: 128,
                height: 128,
                keep_aspect: true,
            };
            s.sample.mode = SampleMode::Box;
            s.dither.mode = DitherMode::Floyd;
            s.adjust.saturation = 1.2;
        },
    },
    Preset {
        name: "console-8bit",
        about: "256 wide, a 32 color console style palette, 2x2 ordered dither",
        build: |s| {
            palette(s, "console-32");
            s.sample.size = Size::Absolute {
                width: 256,
                height: 240,
                keep_aspect: true,
            };
            s.sample.mode = SampleMode::Edge;
            s.dither.mode = DitherMode::Bayer2;
            s.dither.amount = 0.3;
        },
    },
    Preset {
        name: "dungeon",
        about: "320 wide, dark browns and reds, sierra lite for gritty texture",
        build: |s| {
            palette(s, "dungeon-16");
            s.sample.size = Size::Absolute {
                width: 320,
                height: 200,
                keep_aspect: true,
            };
            s.sample.mode = SampleMode::Box;
            s.dither.mode = DitherMode::SierraLite;
            s.adjust.contrast = 1.2;
            s.adjust.gamma = 1.1;
        },
    },
    Preset {
        name: "picocad",
        about: "PICO-8 palette with the picoCAD checker between close colors",
        build: |s| {
            palette(s, "pico-8");
            s.sample.size = Size::Absolute {
                width: 128,
                height: 128,
                keep_aspect: true,
            };
            s.dither.mode = DitherMode::Picocad;
        },
    },
    Preset {
        name: "1-bit",
        about: "black and white with an 8x8 Bayer pattern",
        build: |s| {
            palette(s, "1-bit");
            s.sample.size = Size::Absolute {
                width: 200,
                height: 0,
                keep_aspect: true,
            };
            s.sample.mode = SampleMode::Bilinear;
            s.dither.mode = DitherMode::Bayer8;
            s.dither.amount = 1.0;
            s.colors.metric = Metric::Rgb;
        },
    },
    Preset {
        name: "posterize",
        about: "12 colors taken from the image, no dithering, stray pixels cleaned up",
        build: |s| {
            s.colors.palette = PaletteSource::Extract {
                count: 12,
                method: crate::settings::Quantizer::KMeans,
                locked: Vec::new(),
            };
            s.sample.size = Size::Absolute {
                width: 96,
                height: 0,
                keep_aspect: true,
            };
            s.sample.mode = SampleMode::Edge;
            s.post.cleanup = 2;
        },
    },
];

/// A preset by name.
#[must_use]
pub fn find(name: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_is_valid_and_uses_a_real_palette() {
        for p in PRESETS {
            let s = p.settings();
            s.validate().unwrap();
            if let PaletteSource::Builtin(name) = &s.colors.palette {
                assert!(
                    crate::palette::builtin(name).is_some(),
                    "{}: {name}",
                    p.name
                );
            }
            assert!(!p.about.is_empty());
        }
        assert!(find("pico-8").is_some());
        assert!(find("nope").is_none());
    }
}
