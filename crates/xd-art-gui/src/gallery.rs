//! The dither gallery and the hints on hover.
//!
//! The gallery shows the current image with every dither mode at once, as small
//! previews. They are made from the reduced source on a background thread, one after the
//! other, in the order they appear, and shown as soon as each is done. Closing the gallery
//! or changing a setting cancels whatever is left.
//!
//! Hovering a dither option in the settings panel shows a line about what it does and,
//! once ready, the gallery's preview for it. If the gallery has not made that one yet, it
//! is made on the spot and dropped if the mouse leaves first.

use std::sync::Arc;

use gpui::{AnyElement, Context, SharedString, div, prelude::*, px};
use xd_art_core::buffer::Image;
use xd_art_core::cancel::Cancel;
use xd_art_core::pipeline::Pipeline;
use xd_art_core::settings::{DitherMode, SampleMode, Settings};

use crate::app::Root;
use crate::preview::Viewer;
use crate::theme::{self, Palette};

/// One line about each dither mode.
pub fn dither_hint(m: DitherMode) -> &'static str {
    match m {
        DitherMode::None => "closest color only; flat areas, hard edges",
        DitherMode::Bayer2
        | DitherMode::Bayer3
        | DitherMode::Bayer4
        | DitherMode::Bayer5
        | DitherMode::Bayer8 => "ordered pattern; even texture, stable when the image changes",
        DitherMode::Cluster4 | DitherMode::Cluster8 => "clustered dots, like print halftone",
        DitherMode::Floyd => "error diffusion per channel; smooth gradients, some noise",
        DitherMode::FloydMono => {
            "error diffusion with the error averaged over channels; fewer color specks"
        }
        DitherMode::Stucki | DitherMode::Burkes | DitherMode::Sierra => {
            "error diffusion over a wider area; softer"
        }
        DitherMode::SierraTwoRow | DitherMode::SierraLite => "lighter error diffusion; crisper",
        DitherMode::Picocad => "checker between the two closest colors, only in between",
        DitherMode::MedianCut => "one palette color per region of the image; poster look",
    }
}

/// One line about each sampling mode.
pub fn sample_hint(m: SampleMode) -> &'static str {
    match m {
        SampleMode::Nearest => "one source pixel per output pixel; sharp, can miss thin lines",
        SampleMode::Bilinear => "blend of four pixels; soft",
        SampleMode::Bicubic => "blend of sixteen pixels; soft with some edge contrast",
        SampleMode::Lanczos => "six by six blend; sharpest of the smooth ones",
        SampleMode::Box => "average of the whole area; clean, a bit blurry",
        SampleMode::Cluster => "most common color of the area; keeps flat colors",
        SampleMode::Edge => "area average that keeps edges hard",
    }
}

/// Gallery state kept by the root view.
#[derive(Default)]
pub struct Gallery {
    /// Shown instead of the preview.
    pub open: bool,
    /// One preview per dither mode, in [`DitherMode::ALL`] order.
    pub thumbs: Vec<Option<(Arc<Image>, Viewer)>>,
    cancel: Option<Cancel>,
    /// The dither option under the mouse, and the token of its on demand preview.
    pub hover: Option<(usize, Cancel)>,
}

/// Converts `src` with `s` and the dither mode at `i`.
fn thumb(src: &Image, s: &Settings, i: usize, cancel: &Cancel) -> Option<Image> {
    let mut st = s.clone();
    st.dither.mode = DitherMode::ALL[i];
    st.colors.metric = st.colors.metric.preview();
    st.output.scale = 1;
    let mut p = Pipeline::default();
    p.run(src, 1, &st, cancel).ok()?;
    p.output().cloned()
}

impl Root {
    /// Opens or closes the gallery.
    pub(crate) fn toggle_gallery(&mut self, cx: &mut Context<Self>) {
        self.gallery.open = !self.gallery.open;
        if self.gallery.open {
            self.fill_gallery(cx);
        } else if let Some(c) = self.gallery.cancel.take() {
            c.cancel();
        }
        cx.notify();
    }

    /// Starts making every preview again, after the gallery opened or a setting changed.
    pub(crate) fn fill_gallery(&mut self, cx: &mut Context<Self>) {
        if let Some(c) = self.gallery.cancel.take() {
            c.cancel();
        }
        let Some(src) = self.small_source() else {
            return;
        };
        self.gallery.thumbs = (0..DitherMode::ALL.len()).map(|_| None).collect();
        let cancel = Cancel::new();
        self.gallery.cancel = Some(cancel.clone());
        let settings = self.settings.clone();
        cx.spawn(async move |this, cx| {
            for i in 0..DitherMode::ALL.len() {
                let (src, settings, c) = (src.clone(), settings.clone(), cancel.clone());
                let done = cx
                    .background_executor()
                    .spawn(async move { thumb(&src, &settings, i, &c) })
                    .await;
                if cancel.is_cancelled() {
                    return;
                }
                let Some(img) = done else { continue };
                let ok = this.update(cx, |root, cx| {
                    if let Some(slot) = root.gallery.thumbs.get_mut(i) {
                        let mut v = Viewer::default();
                        v.grid = false;
                        *slot = Some((Arc::new(img), v));
                    }
                    cx.notify();
                });
                if ok.is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    /// The hovered dither option changed. Makes its preview if the gallery does not
    /// have it yet, and cancels the one for the option left behind.
    pub(crate) fn hover_dither(&mut self, i: Option<usize>, cx: &mut Context<Self>) {
        if let Some((_, c)) = self.gallery.hover.take() {
            c.cancel();
        }
        let Some(i) = i else {
            cx.notify();
            return;
        };
        let cancel = Cancel::new();
        self.gallery.hover = Some((i, cancel.clone()));
        let have = self.gallery.thumbs.get(i).is_some_and(Option::is_some);
        if !have && let Some(src) = self.small_source() {
            if self.gallery.thumbs.len() != DitherMode::ALL.len() {
                self.gallery.thumbs = (0..DitherMode::ALL.len()).map(|_| None).collect();
            }
            let settings = self.settings.clone();
            cx.spawn(async move |this, cx| {
                let c = cancel.clone();
                let done = cx
                    .background_executor()
                    .spawn(async move { thumb(&src, &settings, i, &c) })
                    .await;
                if cancel.is_cancelled() {
                    return;
                }
                if let Some(img) = done {
                    this.update(cx, |root, cx| {
                        let mut v = Viewer::default();
                        v.grid = false;
                        root.gallery.thumbs[i] = Some((Arc::new(img), v));
                        cx.notify();
                    })
                    .ok();
                }
            })
            .detach();
        }
        cx.notify();
    }

    /// The gallery grid, shown in place of the preview.
    pub(crate) fn gallery_view(&self, t: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let bg = [t.panel.r, t.panel.g, t.panel.b].map(|v| (v * 255.0).round() as u8);
        let cells: Vec<AnyElement> = DitherMode::ALL
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let current = self.settings.dither.mode == *m;
                let body: AnyElement = match self.gallery.thumbs.get(i).and_then(Option::as_ref) {
                    Some((img, v)) => v.element(img.clone(), None, bg).into_any_element(),
                    None => div().text_color(t.muted).child("...").into_any_element(),
                };
                div()
                    .id(("gallery", i))
                    .w(px(180.0))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_1()
                    .rounded(px(4.0))
                    .bg(t.panel)
                    .border_1()
                    .border_color(if current { t.accent } else { t.border })
                    .cursor_pointer()
                    .hover(|d| d.border_color(t.muted))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings.dither.mode = DitherMode::ALL[i];
                        this.gallery.open = false;
                        this.settings_changed(false, cx);
                    }))
                    .child(
                        div()
                            .h(px(130.0))
                            .w_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(body),
                    )
                    .child(
                        div()
                            .text_size(theme::size::small())
                            .text_color(if current { t.accent } else { t.text })
                            .child(m.name()),
                    )
                    .into_any_element()
            })
            .collect();
        div()
            .id("gallery")
            .size_full()
            .overflow_y_scroll()
            .p_3()
            .flex()
            .flex_wrap()
            .gap_2()
            .content_start()
            .children(cells)
    }

    /// The hint line and preview for the hovered dither option, if any.
    pub(crate) fn dither_hint_view(&self, t: Palette) -> Option<AnyElement> {
        let (i, _) = self.gallery.hover.as_ref()?;
        let bg = [t.elevated.r, t.elevated.g, t.elevated.b].map(|v| (v * 255.0).round() as u8);
        let thumb = self
            .gallery
            .thumbs
            .get(*i)
            .and_then(Option::as_ref)
            .map(|(img, v)| v.element(img.clone(), None, bg));
        Some(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .p_2()
                .rounded(px(4.0))
                .bg(t.elevated)
                .text_size(theme::size::small())
                .text_color(t.muted)
                .child(SharedString::from(dither_hint(DitherMode::ALL[*i])))
                .when_some(thumb, |d, th| d.child(div().h(px(96.0)).w_full().child(th)))
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mode_has_a_hint() {
        for m in DitherMode::ALL {
            assert!(!dither_hint(m).is_empty());
        }
        for m in SampleMode::ALL {
            assert!(!sample_hint(m).is_empty());
        }
    }

    #[test]
    fn thumbs_use_the_requested_mode() {
        let mut img = Image::new(40, 40, [0; 3]).unwrap();
        for x in 0..40 {
            for y in 0..40 {
                img.set(x, y, [(x * 6) as u8, 128, 128, 255]);
            }
        }
        let s = Settings::default();
        let a = thumb(&img, &s, 0, &Cancel::new()).unwrap();
        let b = thumb(
            &img,
            &s,
            DitherMode::ALL
                .iter()
                .position(|m| *m == DitherMode::Bayer8)
                .unwrap(),
            &Cancel::new(),
        )
        .unwrap();
        assert_ne!(a, b);
        let c = Cancel::new();
        c.cancel();
        assert!(thumb(&img, &s, 0, &c).is_none());
    }
}
