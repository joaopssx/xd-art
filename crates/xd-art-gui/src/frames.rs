//! The frame editor for animated GIFs.
//!
//! Opening a GIF lists its frames in a strip under the preview. Click a frame to see it
//! converted, shift click to select a range. The selected frames can be skipped (kept
//! but left out of the export), removed, duplicated or moved, and their duration
//! changed. Play runs the converted animation with each frame's own duration.

use std::sync::Arc;
use std::time::Duration;

use gpui::{AnyElement, ClickEvent, Context, SharedString, div, prelude::*, px};
use xd_art_core::buffer::Image;

use crate::app::{Doc, Root};
use crate::theme::{self, Palette};

/// One frame in the editor.
#[derive(Clone)]
pub struct Frame {
    /// The source pixels.
    pub image: Arc<Image>,
    /// Display time in hundredths of a second, as GIF stores it.
    pub delay: u16,
    /// Left out of the export.
    pub skip: bool,
}

/// The editor's state.
#[derive(Default)]
pub struct Frames {
    /// Every frame, in order.
    pub list: Vec<Frame>,
    /// The frame shown.
    pub current: usize,
    /// Other end of the selected range, with `current`.
    pub anchor: Option<usize>,
    /// The animation is playing.
    pub playing: bool,
}

impl Frames {
    /// Indices of the selected frames, low to high.
    pub fn selection(&self) -> std::ops::RangeInclusive<usize> {
        let a = self.anchor.unwrap_or(self.current);
        a.min(self.current)..=a.max(self.current)
    }

    /// Removes the selection, keeping at least one frame.
    pub fn remove(&mut self) {
        let r = self.selection();
        if r.clone().count() >= self.list.len() {
            return;
        }
        let start = *r.start();
        self.list.drain(r);
        self.current = start.min(self.list.len() - 1);
        self.anchor = None;
    }

    /// Copies the selection right after itself.
    pub fn duplicate(&mut self) {
        let r = self.selection();
        let end = *r.end();
        let copy: Vec<Frame> = self.list[r].to_vec();
        let n = copy.len();
        for (k, f) in copy.into_iter().enumerate() {
            self.list.insert(end + 1 + k, f);
        }
        self.current = end + n;
        self.anchor = Some(end + 1);
    }

    /// Moves the selection one place left (`-1`) or right (`1`).
    pub fn shift(&mut self, dir: isize) {
        let r = self.selection();
        let (s, e) = (*r.start(), *r.end());
        if (dir < 0 && s == 0) || (dir > 0 && e + 1 >= self.list.len()) {
            return;
        }
        if dir < 0 {
            let f = self.list.remove(s - 1);
            self.list.insert(e, f);
        } else {
            let f = self.list.remove(e + 1);
            self.list.insert(s, f);
        }
        self.current = (self.current as isize + dir) as usize;
        self.anchor = self.anchor.map(|a| (a as isize + dir) as usize);
    }

    /// Sets the duration of the selection.
    pub fn set_delay(&mut self, cs: u16) {
        for i in self.selection() {
            self.list[i].delay = cs.max(1);
        }
    }

    /// Skips or unskips the selection.
    pub fn toggle_skip(&mut self) {
        let r = self.selection();
        let to = !self.list[*r.start()].skip;
        for i in r {
            self.list[i].skip = to;
        }
    }

    /// The next frame that is not skipped, after `i`, wrapping around.
    pub fn next_shown(&self, i: usize) -> usize {
        let n = self.list.len();
        (1..=n)
            .map(|k| (i + k) % n)
            .find(|&j| !self.list[j].skip)
            .unwrap_or(i)
    }
}

impl Root {
    /// Shows frame `i`.
    pub(crate) fn show_frame(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some(f) = self.frames.list.get(i) else {
            return;
        };
        let image = f.image.clone();
        self.frames.current = i;
        if let Doc::Ready { image: shown, .. } = &mut self.doc {
            *shown = image;
        }
        // Each frame is its own source for the pipeline cache.
        self.source_id = (self.source_id & !0xffff) | (i as u64 + 1);
        self.convert(self.frames.playing, cx);
        cx.notify();
    }

    fn play(&mut self, cx: &mut Context<Self>) {
        self.frames.playing = !self.frames.playing;
        if !self.frames.playing {
            return;
        }
        cx.spawn(async move |this, cx| {
            loop {
                let delay = this
                    .update(cx, |root, cx| {
                        if !root.frames.playing || root.frames.list.is_empty() {
                            return None;
                        }
                        let next = root.frames.next_shown(root.frames.current);
                        root.show_frame(next, cx);
                        Some(root.frames.list[next].delay)
                    })
                    .ok()
                    .flatten();
                let Some(cs) = delay else { return };
                cx.background_executor()
                    .timer(Duration::from_millis(u64::from(cs.max(2)) * 10))
                    .await;
            }
        })
        .detach();
    }

    fn frames_edit(&mut self, f: impl FnOnce(&mut Frames), cx: &mut Context<Self>) {
        f(&mut self.frames);
        let i = self.frames.current;
        self.show_frame(i, cx);
    }

    /// The frame strip.
    pub(crate) fn frame_strip(&self, t: Palette, cx: &mut Context<Self>) -> AnyElement {
        let sel = self.frames.selection();
        let cells: Vec<AnyElement> = self
            .frames
            .list
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let current = i == self.frames.current;
                let in_sel = sel.contains(&i);
                div()
                    .id(("frame", i))
                    .flex_none()
                    .w(px(46.0))
                    .flex()
                    .flex_col()
                    .items_center()
                    .p(px(2.0))
                    .rounded(px(3.0))
                    .border_1()
                    .border_color(if current {
                        t.accent
                    } else if in_sel {
                        t.muted
                    } else {
                        t.border
                    })
                    .when(f.skip, |d| d.opacity(0.4))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
                        if e.modifiers().shift {
                            this.frames.anchor.get_or_insert(this.frames.current);
                        } else {
                            this.frames.anchor = None;
                        }
                        this.frames.playing = false;
                        this.show_frame(i, cx);
                    }))
                    .child(
                        div()
                            .text_size(theme::size::small())
                            .text_color(t.text)
                            .child(SharedString::from(format!("{}", i + 1))),
                    )
                    .child(
                        div()
                            .text_size(theme::size::small())
                            .text_color(t.muted)
                            .font(theme::number_font())
                            .child(SharedString::from(format!("{}ms", u32::from(f.delay) * 10))),
                    )
                    .into_any_element()
            })
            .collect();
        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .px_2()
                .rounded(px(3.0))
                .cursor_pointer()
                .text_color(t.muted)
                .hover(|d| d.bg(t.elevated).text_color(t.text))
                .child(label)
        };
        let delay = self
            .frames
            .list
            .get(self.frames.current)
            .map_or(0, |f| f.delay);
        div()
            .id("frame-strip")
            .flex_none()
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_2()
            .bg(t.panel)
            .border_t_1()
            .border_color(t.border)
            .text_size(theme::size::small())
            .child(
                div()
                    .id("frames")
                    .flex()
                    .gap_1()
                    .overflow_x_scroll()
                    .children(cells),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        button("play", if self.frames.playing { "stop" } else { "play" })
                            .on_click(cx.listener(|this, _, _, cx| this.play(cx))),
                    )
                    .child(button("skip", "skip").on_click(
                        cx.listener(|this, _, _, cx| this.frames_edit(Frames::toggle_skip, cx)),
                    ))
                    .child(button("dup", "duplicate").on_click(
                        cx.listener(|this, _, _, cx| this.frames_edit(Frames::duplicate, cx)),
                    ))
                    .child(button("remove", "remove").on_click(
                        cx.listener(|this, _, _, cx| this.frames_edit(Frames::remove, cx)),
                    ))
                    .child(button("left", "move left").on_click(
                        cx.listener(|this, _, _, cx| this.frames_edit(|f| f.shift(-1), cx)),
                    ))
                    .child(button("right", "move right").on_click(
                        cx.listener(|this, _, _, cx| this.frames_edit(|f| f.shift(1), cx)),
                    ))
                    .child(button("slower", "longer").on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.frames_edit(|f| f.set_delay(delay.saturating_add(2)), cx);
                        },
                    )))
                    .child(button("faster", "shorter").on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.frames_edit(|f| f.set_delay(delay.saturating_sub(2)), cx);
                        },
                    )))
                    .child(div().text_color(t.muted).child(SharedString::from(format!(
                        "{} frames, {} selected, shift click selects a range",
                        self.frames.list.len(),
                        sel.count()
                    )))),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(n: u8) -> Frames {
        Frames {
            list: (0..n)
                .map(|i| Frame {
                    image: Arc::new(Image::new(1, 1, [i, 0, 0]).unwrap()),
                    delay: 10,
                    skip: false,
                })
                .collect(),
            ..Frames::default()
        }
    }

    fn order(f: &Frames) -> Vec<u8> {
        f.list.iter().map(|f| f.image.rgb()[0]).collect()
    }

    #[test]
    fn edit_ranges() {
        let mut f = frames(5);
        f.current = 1;
        f.anchor = Some(2);
        f.duplicate();
        assert_eq!(order(&f), [0, 1, 2, 1, 2, 3, 4]);
        f.remove();
        assert_eq!(order(&f), [0, 1, 2, 3, 4]);
        f.current = 3;
        f.anchor = None;
        f.shift(-1);
        assert_eq!(order(&f), [0, 1, 3, 2, 4]);
        assert_eq!(f.current, 2);
        f.shift(1);
        assert_eq!(order(&f), [0, 1, 2, 3, 4]);
    }

    #[test]
    fn skipping_and_delays() {
        let mut f = frames(4);
        f.current = 1;
        f.anchor = Some(2);
        f.toggle_skip();
        assert_eq!(f.next_shown(0), 3);
        f.set_delay(0);
        assert_eq!(
            f.list[1].delay, 1,
            "zero would mean no delay at all in some viewers"
        );
        let mut one = frames(1);
        one.remove();
        assert_eq!(one.list.len(), 1, "the last frame stays");
    }
}
