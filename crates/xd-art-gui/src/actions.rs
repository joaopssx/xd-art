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
    ]
);

/// The key context of the main window.
pub const CONTEXT: &str = "xd-art";

/// `(keys on Windows and Linux, keys on macOS, what it does)`.
pub const SHORTCUTS: &[(&str, &str, &str)] = &[
    ("ctrl-o", "cmd-o", "open an image"),
    ("ctrl-[", "cmd-[", "show or hide the settings panel"),
    ("ctrl-]", "cmd-]", "show or hide the information panel"),
];

/// Binds every shortcut.
pub fn bind(cx: &mut App) {
    let key =
        |pc: &'static str, mac: &'static str| if cfg!(target_os = "macos") { mac } else { pc };
    cx.bind_keys([
        KeyBinding::new(key(SHORTCUTS[0].0, SHORTCUTS[0].1), Open, Some(CONTEXT)),
        KeyBinding::new(
            key(SHORTCUTS[1].0, SHORTCUTS[1].1),
            ToggleLeft,
            Some(CONTEXT),
        ),
        KeyBinding::new(
            key(SHORTCUTS[2].0, SHORTCUTS[2].1),
            ToggleRight,
            Some(CONTEXT),
        ),
    ]);
}
