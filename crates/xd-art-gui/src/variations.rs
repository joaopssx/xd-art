//! Variations: up to four versions of the image side by side.
//!
//! Keep the current settings as a variation, change things, keep another, and compare.
//! All of them use the same zoom and position. One can be marked as the favorite,
//! exported straight away, or promoted back to the main settings. The "sizes" button fills
//! the view with the current settings at 32, 64 and 128 pixels wide, made in parallel.

use std::sync::Arc;

use gpui::{AnyElement, Context, SharedString, div, prelude::*, px, svg};
use xd_art_core::buffer::Image;
use xd_art_core::cancel::Cancel;
use xd_art_core::pipeline::Pipeline;
use xd_art_core::settings::{Settings, Size};

use crate::app::Root;
use crate::assets::icon;
use crate::preview::Viewer;
use crate::theme::{self, Palette};

/// Most variations shown at once.
pub const MAX: usize = 4;

/// One kept version.
pub struct Variation {
    /// Shown under it.
    pub label: String,
    /// Its settings.
    pub settings: Settings,
    /// The result, once converted.
    pub image: Option<Arc<Image>>,
    /// Its view.
    pub viewer: Viewer,
}

/// The variations view.
#[derive(Default)]
pub struct Variations {
    /// Shown instead of the preview.
    pub open: bool,
    /// The kept versions, oldest first.
    pub items: Vec<Variation>,
    /// Index of the favorite.
    pub favorite: Option<usize>,
}

fn convert(src: &Image, s: &Settings) -> Option<Image> {
    let mut p = Pipeline::default();
    p.run(src, 1, s, &Cancel::new()).ok()?;
    p.output().cloned()
}

impl Root {
    /// Adds the current settings as a variation and converts it.
    pub(crate) fn keep_variation(
        &mut self,
        label: String,
        settings: Settings,
        cx: &mut Context<Self>,
    ) {
        if self.variations.items.len() == MAX {
            self.variations.items.remove(0);
            self.variations.favorite = self.variations.favorite.and_then(|f| f.checked_sub(1));
        }
        let mut viewer = Viewer::default();
        viewer.grid = false;
        self.variations.items.push(Variation {
            label,
            settings: settings.clone(),
            image: self.output.clone(),
            viewer,
        });
        let index = self.variations.items.len() - 1;
        self.variations.open = true;
        if self.output.is_none() || self.variations.items[index].settings != self.settings {
            self.convert_variation(index, cx);
        }
        cx.notify();
    }

    fn convert_variation(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some(src) = self.full_source() else {
            return;
        };
        let s = self.variations.items[i].settings.clone();
        cx.spawn(async move |this, cx| {
            let out = cx
                .background_executor()
                .spawn(async move { convert(&src, &s) })
                .await;
            this.update(cx, |root, cx| {
                if let (Some(v), Some(img)) = (root.variations.items.get_mut(i), out) {
                    v.image = Some(Arc::new(img));
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Replaces the variations with the current settings at three widths, converted in
    /// parallel.
    pub(crate) fn size_variations(&mut self, cx: &mut Context<Self>) {
        let Some(src) = self.full_source() else {
            return;
        };
        let widths = [32u32, 64, 128];
        self.variations.items = widths
            .iter()
            .map(|w| {
                let mut s = self.settings.clone();
                s.sample.size = Size::Absolute {
                    width: *w,
                    height: 0,
                    keep_aspect: true,
                };
                let mut viewer = Viewer::default();
                viewer.grid = false;
                Variation {
                    label: format!("{w} wide"),
                    settings: s,
                    image: None,
                    viewer,
                }
            })
            .collect();
        self.variations.favorite = None;
        self.variations.open = true;
        let settings = self.settings.clone();
        cx.spawn(async move |this, cx| {
            let out = cx
                .background_executor()
                .spawn(async move {
                    xd_art_core::suggest::previews(&src, &settings, &widths, &Cancel::new())
                })
                .await;
            this.update(cx, |root, cx| {
                if let Ok(imgs) = out {
                    for (v, img) in root.variations.items.iter_mut().zip(imgs) {
                        v.image = Some(Arc::new(img));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn export_variation(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some(img) = self.variations.items.get(i).and_then(|v| v.image.clone()) else {
            return;
        };
        let dir = std::env::current_dir().unwrap_or_default();
        let pick = cx.prompt_for_new_path(&dir, Some("variation.png"));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = pick.await else {
                return;
            };
            let done = cx
                .background_executor()
                .spawn(async move { xd_art_core::io::save(&img, &path).map(|()| path) })
                .await;
            this.update(cx, |root, cx| {
                root.notice = Some(match done {
                    Ok(p) => format!("saved {}", p.display()).into(),
                    Err(e) => e.to_string().into(),
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The variations grid, in place of the preview.
    pub(crate) fn variations_view(&self, t: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let bg = [t.base.r, t.base.g, t.base.b].map(|v| (v * 255.0).round() as u8);
        let (zoom, origin, fitted) = (self.viewer.zoom, self.viewer.origin, self.viewer.fitted);
        let cells: Vec<AnyElement> =
            self.variations
                .items
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let fav = self.variations.favorite == Some(i);
                    let button = |id: &str, label: &'static str| {
                        div()
                            .id(SharedString::from(format!("{id}-{i}")))
                            .px_2()
                            .rounded(px(3.0))
                            .cursor_pointer()
                            .text_color(t.muted)
                            .hover(|d| d.bg(t.elevated).text_color(t.text))
                            .child(label)
                    };
                    div()
                        .flex_1()
                        .min_w(px(200.0))
                        .h_full()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .p_1()
                        .rounded(px(4.0))
                        .border_1()
                        .border_color(if fav { t.accent } else { t.border })
                        .child(div().flex_1().min_h_0().children(
                            // Same zoom and position for all of them, taken from the main view.
                            v.image.clone().map(|img| {
                                v.viewer.element_at((zoom, origin, fitted), img, None, bg)
                            }),
                        ))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .text_size(theme::size::small())
                                .child(
                                    div()
                                        .flex_1()
                                        .text_color(if fav { t.accent } else { t.text })
                                        .child(v.label.clone()),
                                )
                                .child(
                                    button("fav", if fav { "favorite" } else { "mark favorite" })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.variations.favorite =
                                                if fav { None } else { Some(i) };
                                            cx.notify();
                                        })),
                                )
                                .child(button("export", "export").on_click(
                                    cx.listener(move |this, _, _, cx| this.export_variation(i, cx)),
                                ))
                                .child(button("use", "use").on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.settings = this.variations.items[i].settings.clone();
                                        this.variations.open = false;
                                        this.settings_changed(false, cx);
                                    },
                                ))),
                        )
                        .into_any_element()
                })
                .collect();
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .child(
                div()
                    .flex()
                    .gap_2()
                    .text_size(theme::size::small())
                    .text_color(t.muted)
                    .child(
                        div()
                            .id("keep-variation")
                            .flex()
                            .items_center()
                            .gap_1()
                            .px_2()
                            .rounded(px(3.0))
                            .cursor_pointer()
                            .hover(|d| d.bg(t.elevated))
                            .on_click(cx.listener(|this, _, _, cx| {
                                let n = this.variations.items.len() + 1;
                                this.keep_variation(
                                    format!("version {n}"),
                                    this.settings.clone(),
                                    cx,
                                );
                            }))
                            .child(
                                svg()
                                    .path(icon("palette"))
                                    .size(px(12.0))
                                    .text_color(t.muted),
                            )
                            .child("keep current settings"),
                    )
                    .child(
                        div()
                            .id("size-variations")
                            .px_2()
                            .rounded(px(3.0))
                            .cursor_pointer()
                            .hover(|d| d.bg(t.elevated))
                            .on_click(cx.listener(|this, _, _, cx| this.size_variations(cx)))
                            .child("sizes 32, 64, 128"),
                    )
                    .child(format!("{} of {MAX}", self.variations.items.len())),
            )
            .child(div().flex_1().min_h_0().flex().gap_2().children(cells))
    }
}
