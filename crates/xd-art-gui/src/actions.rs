//! Every keyboard shortcut, in one place.
//!
//! Actions are bound here and handled by the root view. The command palette and the
//! shortcut sheet read [`SHORTCUTS`] so what they show cannot drift from what is bound.

use gpui::{App, KeyBinding, actions};

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
];

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
    ]);
}
