//! Every keyboard shortcut, in one place.
//!
//! Actions are bound here and handled by the root view. The command palette and the
//! shortcut sheet read [`SHORTCUTS`] so what they show cannot drift from what is bound.

use gpui::{Action, App, KeyBinding, actions};

actions!(
    xd_art,
    [
        /// Open an image.
        Open,
        /// Show or hide the settings panel.
        ToggleLeft,
        /// Show or hide the information panel.
        ToggleRight,
        /// Fit the image into the view.
        Fit,
        /// Show the image at 100%.
        ActualSize,
        /// Zoom in one level.
        ZoomIn,
        /// Zoom out one level.
        ZoomOut,
        /// Show or hide the pixel grid.
        ToggleGrid,
        /// Cycle the before and after comparison: divider, source only, off.
        Compare,
        /// Show every dither mode side by side.
        Gallery,
        /// Undo the last change.
        Undo,
        /// Redo the change undone last.
        Redo,
        /// Show or hide the variations side by side.
        Variations,
        /// Keep the current settings as a variation.
        KeepVariation,
        /// Search and run any command.
        CommandPalette,
        /// List every shortcut.
        ShortcutSheet,
        /// Switch to the next theme.
        NextTheme,
        /// Take the accent color from the image, or go back to the theme's.
        AccentFromImage,
        /// Only the image, on the whole screen.
        Fullscreen,
        /// Turn transitions off or on.
        ReduceMotion,
        /// Export, recipes and palette import.
        ExportPanel,
        /// Startup and conversion times over the preview.
        DevOverlay,
        /// Stay resident after closing, from the next start.
        ToggleResident,
        /// Quit, also from a resident instance.
        Quit,
    ]
);

/// The key context of the main window.
pub const CONTEXT: &str = "xd-art";

/// `(keys on Windows and Linux, keys on macOS, what it does)`.
pub const SHORTCUTS: &[(&str, &str, &str)] = &[
    ("ctrl-o", "cmd-o", "open an image"),
    ("ctrl-[", "cmd-[", "show or hide the settings panel"),
    ("ctrl-]", "cmd-]", "show or hide the information panel"),
    ("ctrl-0", "cmd-0", "fit the image into the view"),
    ("ctrl-1", "cmd-1", "show the image at 100%"),
    ("ctrl-=", "cmd-=", "zoom in"),
    ("ctrl--", "cmd--", "zoom out"),
    ("ctrl-g", "cmd-g", "show or hide the pixel grid"),
    (
        "ctrl-b",
        "cmd-b",
        "before and after: divider, source only, off",
    ),
    ("ctrl-d", "cmd-d", "dither gallery"),
    ("ctrl-z", "cmd-z", "undo"),
    ("ctrl-shift-z", "cmd-shift-z", "redo"),
    ("ctrl-e", "cmd-e", "variations side by side"),
    (
        "ctrl-shift-e",
        "cmd-shift-e",
        "keep current settings as a variation",
    ),
    ("ctrl-k", "cmd-k", "command palette"),
    ("ctrl-/", "cmd-/", "list every shortcut"),
    (
        "ctrl-t",
        "cmd-t",
        "next theme: dark, darker, midnight, plum",
    ),
    (
        "ctrl-shift-t",
        "cmd-shift-t",
        "accent color from the image on or off",
    ),
    (
        "f11",
        "cmd-ctrl-f",
        "image only, full screen (escape leaves)",
    ),
    ("ctrl-shift-m", "cmd-shift-m", "transitions off or on"),
    ("ctrl-s", "cmd-s", "export, recipes and palette import"),
    (
        "ctrl-shift-d",
        "cmd-shift-d",
        "startup and conversion times",
    ),
    (
        "ctrl-shift-r",
        "cmd-shift-r",
        "stay resident after closing, on or off",
    ),
    ("ctrl-q", "cmd-q", "quit"),
];

/// The action of [`SHORTCUTS`] entry `i`, to run it from the command palette.
pub fn command(i: usize) -> Box<dyn Action> {
    match i {
        0 => Box::new(Open),
        1 => Box::new(ToggleLeft),
        2 => Box::new(ToggleRight),
        3 => Box::new(Fit),
        4 => Box::new(ActualSize),
        5 => Box::new(ZoomIn),
        6 => Box::new(ZoomOut),
        7 => Box::new(ToggleGrid),
        8 => Box::new(Compare),
        9 => Box::new(Gallery),
        10 => Box::new(Undo),
        11 => Box::new(Redo),
        12 => Box::new(Variations),
        13 => Box::new(KeepVariation),
        14 => Box::new(CommandPalette),
        15 => Box::new(ShortcutSheet),
        16 => Box::new(NextTheme),
        17 => Box::new(AccentFromImage),
        18 => Box::new(Fullscreen),
        19 => Box::new(ReduceMotion),
        20 => Box::new(ExportPanel),
        21 => Box::new(DevOverlay),
        22 => Box::new(ToggleResident),
        _ => Box::new(Quit),
    }
}

/// Binds every shortcut.
pub fn bind(cx: &mut App) {
    let key =
        |pc: &'static str, mac: &'static str| if cfg!(target_os = "macos") { mac } else { pc };
    let k = |i: usize| key(SHORTCUTS[i].0, SHORTCUTS[i].1);
    let c = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new(k(0), Open, c),
        KeyBinding::new(k(1), ToggleLeft, c),
        KeyBinding::new(k(2), ToggleRight, c),
        KeyBinding::new(k(3), Fit, c),
        KeyBinding::new(k(4), ActualSize, c),
        KeyBinding::new(k(5), ZoomIn, c),
        KeyBinding::new(k(6), ZoomOut, c),
        KeyBinding::new(k(7), ToggleGrid, c),
        KeyBinding::new(k(8), Compare, c),
        KeyBinding::new(k(9), Gallery, c),
        KeyBinding::new(k(10), Undo, c),
        KeyBinding::new(k(11), Redo, c),
        KeyBinding::new(k(12), Variations, c),
        KeyBinding::new(k(13), KeepVariation, c),
        KeyBinding::new(k(14), CommandPalette, c),
        KeyBinding::new(k(15), ShortcutSheet, c),
        KeyBinding::new(k(16), NextTheme, c),
        KeyBinding::new(k(17), AccentFromImage, c),
        KeyBinding::new(k(18), Fullscreen, c),
        KeyBinding::new(k(19), ReduceMotion, c),
        KeyBinding::new(k(20), ExportPanel, c),
        KeyBinding::new(k(21), DevOverlay, c),
        KeyBinding::new(k(22), ToggleResident, c),
        KeyBinding::new(k(23), Quit, c),
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shortcut_has_a_command() {
        // command() falls through to the last action, so the list and the match have
        // to stay the same length; the names make a mismatch obvious.
        assert_eq!(command(SHORTCUTS.len() - 1).name(), Quit.name());
        assert_eq!(command(0).name(), Open.name());
    }
}
