//! The palette strip under the preview.
//!
//! Every color of the palette in use, in order, with how many pixels use it. Click a
//! color to type a new value, drag it onto another to reorder, right click for lock,
//! duplicate and remove. Locked colors show a small mark and survive rebuilding the
//! palette from the image.
//!
//! Editing a built-in palette turns it into a palette of its own; the built-in stays as
//! it is.

use gpui::{
    AnyElement, Context, KeyDownEvent, MouseButton, Rgba, SharedString, Window, div, prelude::*,
    px, rgb, svg,
};
use xd_art_core::palette::Palette;
use xd_art_core::settings::{PaletteSource, Quantizer};

use crate::app::Root;
use crate::assets::icon;
use crate::theme::{self, Palette as Colors};
use xd_art_text::{fill, mark, tr};

/// A swatch being dragged.
#[derive(Clone, Copy, Debug)]
pub struct DraggedSwatch(pub usize);

impl Render for DraggedSwatch {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// Palette strip state kept by the root view.
#[derive(Debug, Default)]
pub struct Strip {
    /// Colors of the last conversion.
    pub colors: Vec<[u8; 3]>,
    /// Pixels per color in the last conversion.
    pub counts: Vec<u64>,
    /// Swatch whose value is being typed.
    pub editing: Option<(usize, String)>,
    /// Swatch with the menu open.
    pub menu: Option<usize>,
    /// Swatch under the mouse.
    pub hover: Option<usize>,
}

/// `#rrggbb`.
pub fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn color(c: [u8; 3]) -> Rgba {
    rgb(u32::from(c[0]) << 16 | u32::from(c[1]) << 8 | u32::from(c[2]))
}

fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let t = s.trim().trim_start_matches('#');
    (t.len() == 6)
        .then(|| u32::from_str_radix(t, 16).ok())
        .flatten()
        .map(|v| [(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

impl Root {
    /// The palette being edited, as an editable list: the settings' own colors, the
    /// built-in one, or the colors the last conversion extracted.
    fn editable(&self) -> Palette {
        let colors = match &self.settings.colors.palette {
            PaletteSource::Colors(c) => c.clone(),
            PaletteSource::Builtin(name) => xd_art_core::palette::builtin(name)
                .map(|p| p.colors().to_vec())
                .unwrap_or_default(),
            PaletteSource::Extract { .. } => self.strip.colors.clone(),
        };
        Palette::new("edited", colors).unwrap_or_else(|_| {
            Palette::new("edited", vec![[0; 3]]).unwrap_or_else(|_| unreachable!())
        })
    }

    fn locked(&self) -> Vec<[u8; 3]> {
        match &self.settings.colors.palette {
            PaletteSource::Extract { locked, .. } => locked.clone(),
            _ => Vec::new(),
        }
    }

    /// Replaces the palette with `p` and converts again.
    fn set_palette(&mut self, p: &Palette, cx: &mut Context<Self>) {
        self.settings.colors.palette = PaletteSource::Colors(p.colors().to_vec());
        self.settings_changed(false, cx);
    }

    fn toggle_lock(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some(&c) = self.strip.colors.get(i) else {
            return;
        };
        match &mut self.settings.colors.palette {
            PaletteSource::Extract { locked, .. } => {
                if let Some(pos) = locked.iter().position(|l| *l == c) {
                    locked.remove(pos);
                } else {
                    locked.push(c);
                }
            }
            // Locking only means something for a palette rebuilt from the image, so
            // locking a color of a fixed palette switches to extraction with that
            // color kept.
            other => {
                let count = self.strip.colors.len().clamp(2, 256) as u16;
                *other = PaletteSource::Extract {
                    count,
                    method: Quantizer::KMeans,
                    locked: vec![c],
                };
            }
        }
        self.settings_changed(false, cx);
    }

    fn edit_key(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some((i, text)) = &mut self.strip.editing else {
            return;
        };
        let i = *i;
        match e.keystroke.key.as_str() {
            "enter" => {
                if let Some(c) = parse_hex(text) {
                    let mut p = self.editable();
                    p.set(i, c);
                    self.strip.editing = None;
                    self.set_palette(&p, cx);
                }
            }
            "escape" => self.strip.editing = None,
            "backspace" => {
                text.pop();
            }
            _ => {
                if let Some(ch) = e.keystroke.key_char.as_deref()
                    && ch.chars().all(|c| c.is_ascii_hexdigit() || c == '#')
                    && text.len() < 7
                {
                    text.push_str(ch);
                }
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn menu_action(&mut self, i: usize, what: &str, cx: &mut Context<Self>) {
        self.strip.menu = None;
        let mut p = self.editable();
        match what {
            "lock" => return self.toggle_lock(i, cx),
            "duplicate" => {
                if let Some(&c) = p.colors().get(i)
                    && p.push(c).is_ok()
                {
                    p.move_to(p.len() - 1, i + 1);
                }
            }
            "remove" => {
                if p.remove(i).is_err() {
                    return;
                }
            }
            "sort-luma" => p.sort_by_luminance(),
            "sort-hue" => p.sort_by_hue(),
            _ => return,
        }
        self.set_palette(&p, cx);
    }

    /// The strip element.
    pub(crate) fn palette_strip(&self, t: Colors, cx: &mut Context<Self>) -> impl IntoElement {
        let locked = self.locked();
        let total: u64 = self.strip.counts.iter().sum::<u64>().max(1);
        let swatches: Vec<AnyElement> = self
            .strip
            .colors
            .iter()
            .enumerate()
            .map(|(i, &c)| {
                let is_locked = locked.contains(&c);
                let editing = self
                    .strip
                    .editing
                    .as_ref()
                    .filter(|(e, _)| *e == i)
                    .map(|(_, s)| s.clone());
                div()
                    .id(("swatch", i))
                    .relative()
                    .size(px(24.0))
                    .flex_none()
                    .rounded(px(3.0))
                    .bg(color(c))
                    .border_1()
                    .border_color(
                        if editing.is_some()
                            || self.strip.menu == Some(i)
                            || (self.retouch.tool == crate::retouch::Tool::Pencil
                                && self.retouch.color == Some(c))
                        {
                            t.accent
                        } else {
                            t.border
                        },
                    )
                    .cursor_pointer()
                    .on_hover(cx.listener(move |this, on: &bool, _, cx| {
                        this.strip.hover = if *on {
                            Some(i)
                        } else {
                            this.strip.hover.filter(|h| *h != i)
                        };
                        cx.notify();
                    }))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let current = this.strip.colors.get(i).copied().unwrap_or_default();
                        if matches!(
                            this.retouch.tool,
                            crate::retouch::Tool::Pencil | crate::retouch::Tool::Picker
                        ) {
                            this.retouch.color = Some(current);
                            this.retouch.tool = crate::retouch::Tool::Pencil;
                            cx.notify();
                            return;
                        }
                        this.strip.editing = Some((i, hex(current)));
                        this.strip.menu = None;
                        window.focus(&this.focus_handle());
                        cx.notify();
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, _, _, cx| {
                            this.strip.menu = Some(i);
                            this.strip.editing = None;
                            cx.notify();
                        }),
                    )
                    .on_drag(DraggedSwatch(i), |d, _, _, cx| cx.new(|_| *d))
                    .drag_over::<DraggedSwatch>(move |s, _, _, _| {
                        s.border_2().border_color(t.accent)
                    })
                    .on_drop(cx.listener(move |this, d: &DraggedSwatch, _, cx| {
                        let mut p = this.editable();
                        p.move_to(d.0, i);
                        this.set_palette(&p, cx);
                    }))
                    .when(is_locked, |d| {
                        d.child(
                            div()
                                .absolute()
                                .right(px(1.0))
                                .bottom(px(1.0))
                                .size(px(9.0))
                                .rounded(px(2.0))
                                .bg(t.base)
                                .child(svg().path(icon("lock")).size(px(9.0)).text_color(t.text)),
                        )
                    })
                    .into_any_element()
            })
            .collect();
        let info: SharedString = if let Some((i, text)) = &self.strip.editing {
            fill(
                tr("color {0}: {1}   enter applies, escape cancels"),
                &[&i, text],
            )
            .into()
        } else if let Some(i) = self.strip.hover.filter(|i| *i < self.strip.colors.len()) {
            let n = self.strip.counts.get(i).copied().unwrap_or(0);
            fill(
                tr("{0}   {1} pixels, {2}%"),
                &[
                    &hex(self.strip.colors[i]),
                    &n,
                    &format!("{:.1}", n as f64 * 100.0 / total as f64),
                ],
            )
            .into()
        } else {
            fill(tr("{0} colors"), &[&self.strip.colors.len()]).into()
        };
        let menu = self.strip.menu.map(|i| {
            let item = |label: &'static str, what: &'static str| {
                div()
                    .id(SharedString::from(format!("menu-{what}")))
                    .px_2()
                    .py(px(2.0))
                    .rounded(px(3.0))
                    .cursor_pointer()
                    .hover(|d| d.bg(t.border))
                    .on_click(cx.listener(move |this, _, _, cx| this.menu_action(i, what, cx)))
                    .child(tr(label))
            };
            div()
                .flex()
                .gap_1()
                .p_1()
                .rounded(px(4.0))
                .bg(t.elevated)
                .border_1()
                .border_color(t.border)
                .child(item(
                    if locked.contains(&self.strip.colors.get(i).copied().unwrap_or_default()) {
                        mark("unlock")
                    } else {
                        mark("lock")
                    },
                    "lock",
                ))
                .child(item(mark("duplicate"), "duplicate"))
                .child(item(mark("remove"), "remove"))
                .child(item(mark("sort by lightness"), "sort-luma"))
                .child(item(mark("sort by hue"), "sort-hue"))
        });
        div()
            .id("palette-strip")
            .flex_none()
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_2()
            .bg(t.panel)
            .border_t_1()
            .border_color(t.border)
            .on_key_down(cx.listener(Self::edit_key))
            .child(div().flex().flex_wrap().gap_1().children(swatches))
            .child(
                div()
                    .flex()
                    .gap_3()
                    .items_center()
                    .text_size(theme::size::small())
                    .text_color(t.muted)
                    .font(theme::number_font())
                    .child(info)
                    .children(menu),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        assert_eq!(hex([255, 8, 128]), "#ff0880");
        assert_eq!(parse_hex("#ff0880"), Some([255, 8, 128]));
        assert_eq!(parse_hex("ff0880"), Some([255, 8, 128]));
        assert_eq!(parse_hex("#ff08"), None);
        assert_eq!(parse_hex("zzzzzz"), None);
    }
}
