//! History of settings: undo, redo and a timeline with small previews.
//!
//! Every finished change (not every step of a slider drag) adds an entry named after
//! what changed. Undo and redo move along the list; clicking an entry jumps to it. Going
//! back does not throw anything away: the later entries stay until a new change is
//! made, which then replaces them.

use std::sync::Arc;

use gpui::{AnyElement, Context, SharedString, div, prelude::*, px};
use xd_art_core::buffer::Image;
use xd_art_core::settings::{PaletteSource, Settings, Size};

use crate::app::Root;
use crate::preview::Viewer;
use crate::theme::{self, Palette};
use xd_art_text::{fill, mark, tr};

/// Most entries kept.
pub const MAX_ENTRIES: usize = 200;
/// Most bytes of previews kept; older entries lose their preview first.
pub const MAX_PREVIEW_BYTES: usize = 64 << 20;

/// One state in the history.
pub struct Entry {
    /// What changed, like "dithering: sierra".
    pub name: String,
    /// The settings after the change.
    pub settings: Settings,
    /// The result, once converted.
    pub preview: Option<(Arc<Image>, Viewer)>,
}

/// The history list and the current position in it.
#[derive(Default)]
pub struct History {
    /// Oldest first.
    pub entries: Vec<Entry>,
    /// Index of the current entry.
    pub pos: usize,
}

fn f(v: f32) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// A short name for the difference between two settings.
pub fn describe(old: &Settings, new: &Settings) -> String {
    if old.dither.mode != new.dither.mode {
        return fill(tr("dithering: {0}"), &[&new.dither.mode.name()]);
    }
    if old.colors.palette != new.colors.palette {
        return match &new.colors.palette {
            PaletteSource::Builtin(n) => fill(tr("palette: {0}"), &[n]),
            PaletteSource::Colors(c) => fill(tr("palette: {0} colors"), &[&c.len()]),
            PaletteSource::Extract { count, .. } => {
                fill(tr("palette: {0} from the image"), &[count])
            }
        };
    }
    if old.colors.metric != new.colors.metric {
        return fill(tr("distance: {0}"), &[&new.colors.metric.name()]);
    }
    if old.sample.mode != new.sample.mode {
        return fill(tr("sampling: {0}"), &[&new.sample.mode.name()]);
    }
    if old.sample.size != new.sample.size {
        return match new.sample.size {
            Size::Absolute {
                width,
                height,
                keep_aspect: false,
            } => fill(tr("size: {0} x {1}"), &[&width, &height]),
            Size::Absolute { width, .. } => fill(tr("size: {0} wide"), &[&width]),
            Size::Relative { x, .. } => fill(tr("size: divided by {0}"), &[&x]),
        };
    }
    let a = (&old.adjust, &new.adjust);
    for (name, x, y) in [
        (mark("brightness"), a.0.brightness, a.1.brightness),
        (mark("contrast"), a.0.contrast, a.1.contrast),
        (mark("saturation"), a.0.saturation, a.1.saturation),
        (mark("hue"), a.0.hue, a.1.hue),
        (mark("gamma"), a.0.gamma, a.1.gamma),
        (mark("sharpen"), a.0.sharpen, a.1.sharpen),
        (mark("blur"), old.blur.radius, new.blur.radius),
        (mark("dither amount"), old.dither.amount, new.dither.amount),
        (mark("offset x"), old.sample.offset_x, new.sample.offset_x),
        (mark("offset y"), old.sample.offset_y, new.sample.offset_y),
    ] {
        if x != y {
            return format!("{}: {}", tr(name), f(y));
        }
    }
    if old.post != new.post {
        return tr("post-processing").into();
    }
    if old.output != new.output {
        return fill(tr("scale: {0}x"), &[&new.output.scale]);
    }
    if old.mask != new.mask {
        return if new.mask.is_some() {
            tr("dither mask").into()
        } else {
            tr("mask cleared").into()
        };
    }
    tr("settings").into()
}

impl History {
    /// Starts the history at `s`.
    pub fn start(&mut self, s: &Settings) {
        self.entries = vec![Entry {
            name: tr("opened").into(),
            settings: s.clone(),
            preview: None,
        }];
        self.pos = 0;
    }

    /// Records `s` after the current entry, dropping anything after it.
    pub fn record(&mut self, s: &Settings) {
        let Some(cur) = self.entries.get(self.pos) else {
            return self.start(s);
        };
        if cur.settings == *s {
            return;
        }
        let name = describe(&cur.settings, s);
        self.entries.truncate(self.pos + 1);
        self.entries.push(Entry {
            name,
            settings: s.clone(),
            preview: None,
        });
        if self.entries.len() > MAX_ENTRIES {
            self.entries.remove(0);
        }
        self.pos = self.entries.len() - 1;
    }

    /// Moves back one entry; the settings to apply, if there was one.
    pub fn undo(&mut self) -> Option<Settings> {
        (self.pos > 0).then(|| {
            self.pos -= 1;
            self.entries[self.pos].settings.clone()
        })
    }

    /// Moves forward one entry.
    pub fn redo(&mut self) -> Option<Settings> {
        (self.pos + 1 < self.entries.len()).then(|| {
            self.pos += 1;
            self.entries[self.pos].settings.clone()
        })
    }

    /// Stores the result of the current entry, keeping previews within the limit.
    pub fn set_preview(&mut self, img: Arc<Image>) {
        let Some(e) = self.entries.get_mut(self.pos) else {
            return;
        };
        let mut v = Viewer::default();
        v.grid = false;
        e.preview = Some((img, v));
        let mut total = 0usize;
        for e in self.entries.iter_mut().rev() {
            if let Some((img, _)) = &e.preview {
                total += img.heap_size();
                if total > MAX_PREVIEW_BYTES {
                    e.preview = None;
                }
            }
        }
    }
}

impl Root {
    /// Applies settings taken from the history, without recording them again.
    pub(crate) fn restore(&mut self, s: Settings, cx: &mut Context<Self>) {
        self.settings = s;
        self.restoring = true;
        self.settings_changed(false, cx);
        self.restoring = false;
    }

    /// The timeline, newest at the top.
    pub(crate) fn timeline(&self, t: Palette, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let bg = [t.panel.r, t.panel.g, t.panel.b].map(|v| (v * 255.0).round() as u8);
        let h = &self.history;
        h.entries
            .iter()
            .enumerate()
            .rev()
            .map(|(i, e)| {
                let current = i == h.pos;
                let later = i > h.pos;
                div()
                    .id(("history", i))
                    .flex()
                    .items_center()
                    .gap_2()
                    .p_1()
                    .rounded(px(4.0))
                    .when(current, |d| d.bg(t.elevated))
                    .cursor_pointer()
                    .hover(|d| d.bg(t.elevated))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.history.pos = i;
                        let s = this.history.entries[i].settings.clone();
                        this.restore(s, cx);
                    }))
                    .child(
                        div()
                            .size(px(36.0))
                            .flex_none()
                            .rounded(px(3.0))
                            .bg(t.base)
                            .children(
                                e.preview
                                    .as_ref()
                                    .map(|(img, v)| v.element(img.clone(), None, bg)),
                            ),
                    )
                    .child(
                        div()
                            .text_size(theme::size::small())
                            .text_color(if current {
                                t.text
                            } else if later {
                                t.border
                            } else {
                                t.muted
                            })
                            .child(SharedString::from(e.name.clone())),
                    )
                    .into_any_element()
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xd_art_core::settings::DitherMode;

    #[test]
    fn names_say_what_changed() {
        let a = Settings::default();
        let mut b = a.clone();
        b.dither.mode = DitherMode::Sierra;
        assert_eq!(describe(&a, &b), "dithering: sierra");
        let mut c = a.clone();
        c.adjust.brightness = 0.1;
        assert_eq!(describe(&a, &c), "brightness: 0.1");
        let mut d = a.clone();
        d.colors.palette = PaletteSource::Builtin("cga".into());
        assert_eq!(describe(&a, &d), "palette: cga");
    }

    #[test]
    fn undo_redo_and_branching() {
        let mut h = History::default();
        let s0 = Settings::default();
        h.start(&s0);
        let mut s1 = s0.clone();
        s1.adjust.hue = 10.0;
        h.record(&s1);
        let mut s2 = s1.clone();
        s2.adjust.hue = 20.0;
        h.record(&s2);
        assert_eq!(h.undo(), Some(s1.clone()));
        assert_eq!(h.undo(), Some(s0.clone()));
        assert_eq!(h.undo(), None);
        assert_eq!(h.redo(), Some(s1.clone()));
        // Later entries survive going back...
        assert_eq!(h.entries.len(), 3);
        // ...until a new change replaces them.
        let mut s3 = s1.clone();
        s3.adjust.gamma = 2.0;
        h.record(&s3);
        assert_eq!(h.entries.len(), 3);
        assert_eq!(h.entries[2].name, "gamma: 2");
        assert_eq!(h.redo(), None);
        h.record(&s3);
        assert_eq!(h.entries.len(), 3, "same settings are not recorded twice");
    }

    #[test]
    fn history_is_bounded() {
        let mut h = History::default();
        let mut s = Settings::default();
        h.start(&s);
        for i in 0..(MAX_ENTRIES + 20) {
            s.adjust.hue = i as f32;
            h.record(&s);
        }
        assert_eq!(h.entries.len(), MAX_ENTRIES);
        assert_eq!(h.pos, MAX_ENTRIES - 1);
    }
}
