//! The command palette (Ctrl+K) and the shortcut sheet (Ctrl+/).
//!
//! Both read [`crate::actions::SHORTCUTS`], the same list the key bindings come from, so
//! a command listed here always has the shortcut shown next to it.

use gpui::{AnyElement, Context, KeyDownEvent, SharedString, Window, div, prelude::*, px};

use crate::actions::{self, SHORTCUTS};
use crate::app::Root;
use crate::theme::{self, Palette};

/// State of the palette.
#[derive(Debug, Default)]
pub struct Commands {
    /// The palette is open.
    pub open: bool,
    /// The shortcut sheet is open.
    pub sheet: bool,
    /// Typed filter.
    pub query: String,
    /// Highlighted row among the matches.
    pub selected: usize,
}

/// Indices into [`SHORTCUTS`] whose description contains every word of `query`.
pub fn matches(query: &str) -> Vec<usize> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    (0..SHORTCUTS.len())
        .filter(|&i| words.iter().all(|w| SHORTCUTS[i].2.contains(w.as_str())))
        .collect()
}

/// The keys of a shortcut as shown on this platform.
pub fn keys(i: usize) -> String {
    let k = if cfg!(target_os = "macos") {
        SHORTCUTS[i].1
    } else {
        SHORTCUTS[i].0
    };
    // "ctrl--" is Ctrl and the minus key, so the key is split off from the end.
    let (mods, key) = match k.strip_suffix("--") {
        Some(m) => (m, "-"),
        None => k.rsplit_once('-').unwrap_or(("", k)),
    };
    let mut parts: Vec<String> = mods
        .split('-')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut c = p.chars();
            c.next()
                .map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
        })
        .collect();
    parts.push(key.to_uppercase());
    parts.join("+")
}

impl Root {
    /// Keys typed while the palette is open.
    pub(crate) fn command_key(
        &mut self,
        e: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.commands.open {
            if self.commands.sheet && e.keystroke.key == "escape" {
                self.commands.sheet = false;
                cx.notify();
            }
            return;
        }
        let found = matches(&self.commands.query);
        match e.keystroke.key.as_str() {
            "escape" => self.commands.open = false,
            "enter" => {
                if let Some(&i) = found.get(self.commands.selected) {
                    self.commands.open = false;
                    window.dispatch_action(actions::command(i), cx);
                }
            }
            "up" => self.commands.selected = self.commands.selected.saturating_sub(1),
            "down" => {
                self.commands.selected =
                    (self.commands.selected + 1).min(found.len().saturating_sub(1))
            }
            "backspace" => {
                self.commands.query.pop();
                self.commands.selected = 0;
            }
            _ => {
                if let Some(c) = e.keystroke.key_char.as_deref() {
                    self.commands.query.push_str(c);
                    self.commands.selected = 0;
                } else {
                    return;
                }
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// The palette overlay.
    pub(crate) fn command_palette(&self, t: Palette, cx: &mut Context<Self>) -> AnyElement {
        let found = matches(&self.commands.query);
        let rows: Vec<AnyElement> = found
            .iter()
            .enumerate()
            .map(|(row, &i)| {
                let sel = row == self.commands.selected;
                div()
                    .id(("command", i))
                    .flex()
                    .justify_between()
                    .px_2()
                    .py_1()
                    .rounded(px(3.0))
                    .when(sel, |d| d.bg(t.elevated))
                    .cursor_pointer()
                    .hover(|d| d.bg(t.elevated))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.commands.open = false;
                        window.dispatch_action(actions::command(i), cx);
                    }))
                    .child(
                        div()
                            .text_color(if sel { t.text } else { t.muted })
                            .child(SHORTCUTS[i].2),
                    )
                    .child(
                        div()
                            .text_color(t.muted)
                            .font(theme::number_font())
                            .child(keys(i)),
                    )
                    .into_any_element()
            })
            .collect();
        overlay(t)
            .child(
                div()
                    .px_2()
                    .py_1()
                    .rounded(px(4.0))
                    .bg(t.base)
                    .border_1()
                    .border_color(t.accent)
                    .text_color(t.text)
                    .child(SharedString::from(format!("{}|", self.commands.query))),
            )
            .children(rows)
            .into_any_element()
    }

    /// Every shortcut in a table.
    pub(crate) fn shortcut_sheet(&self, t: Palette) -> AnyElement {
        overlay(t)
            .child(div().text_size(theme::size::title()).child("Shortcuts"))
            .children((0..SHORTCUTS.len()).map(|i| {
                div()
                    .flex()
                    .justify_between()
                    .gap_4()
                    .child(div().text_color(t.muted).child(SHORTCUTS[i].2))
                    .child(div().font(theme::number_font()).child(keys(i)))
            }))
            .child(
                div()
                    .mt_2()
                    .text_size(theme::size::small())
                    .text_color(t.muted)
                    .child("escape closes"),
            )
            .into_any_element()
    }
}

fn overlay(t: Palette) -> gpui::Div {
    div()
        .absolute()
        .top(px(60.0))
        .left_1_2()
        .ml(px(-220.0))
        .w(px(440.0))
        .flex()
        .flex_col()
        .gap_1()
        .p_2()
        .rounded(px(6.0))
        .bg(t.panel)
        .border_1()
        .border_color(t.border)
        .text_size(theme::size::body())
        .text_color(t.text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filtering_by_words() {
        assert_eq!(matches("").len(), SHORTCUTS.len());
        let z = matches("zoom in");
        assert_eq!(z.len(), 1);
        assert_eq!(SHORTCUTS[z[0]].2, "zoom in");
        assert!(matches("panel").len() >= 2);
        assert!(matches("nothing like this").is_empty());
    }

    #[test]
    fn key_labels() {
        let i = SHORTCUTS.iter().position(|s| s.2 == "redo").unwrap();
        let k = keys(i);
        assert!(k == "Ctrl+Shift+Z" || k == "Cmd+Shift+Z", "{k}");
        let i = SHORTCUTS.iter().position(|s| s.2 == "zoom out").unwrap();
        assert!(keys(i).ends_with("+-"), "{}", keys(i));
    }
}
