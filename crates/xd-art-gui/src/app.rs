//! The root view: top bar, side panels around the preview, status bar and footer.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use gpui::{
    AnyElement, Context, MouseButton, MouseMoveEvent, Pixels, SharedString, Window, div,
    prelude::*, px, svg,
};
use xd_art_core::buffer::Image;

use crate::Marks;
use crate::assets::icon;
use crate::theme::{self, Palette, Theme};

/// What the window is showing.
pub enum Doc {
    /// Nothing open yet.
    Empty,
    /// A file is being decoded.
    Loading(PathBuf),
    /// A decoded image.
    Ready {
        /// Where it came from.
        path: PathBuf,
        /// The pixels.
        image: Arc<Image>,
    },
    /// Opening failed.
    Failed(String),
}

/// A side panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// Settings, in pipeline order.
    Left,
    /// Information and tools.
    Right,
}

const PANEL_MIN: f32 = 200.0;
const PANEL_MAX: f32 = 520.0;
const FOOTER_URL: &str = "https://github.com/joaopssx";

/// Panel sizes and visibility. Stored in the config file.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    /// Left panel shown.
    pub left_open: bool,
    /// Right panel shown.
    pub right_open: bool,
    /// Left panel width in logical pixels.
    pub left_width: f32,
    /// Right panel width in logical pixels.
    pub right_width: f32,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            left_open: true,
            right_open: true,
            left_width: 290.0,
            right_width: 250.0,
        }
    }
}

/// The application window's content.
pub struct Root {
    marks: Marks,
    doc: Doc,
    layout: Layout,
    /// The panel edge being dragged, if any.
    resizing: Option<Side>,
}

impl Root {
    /// An empty view; `marks` carries the startup times so far.
    pub fn new(mut marks: Marks) -> Self {
        marks.window = Some(Instant::now());
        Self {
            marks,
            doc: Doc::Empty,
            layout: Layout::default(),
            resizing: None,
        }
    }

    /// Records when the first frame was on screen.
    pub fn first_frame(&mut self) {
        self.marks.first_frame.get_or_insert_with(Instant::now);
    }

    /// Decodes `path` on a background thread and shows it when done.
    pub fn open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.doc = Doc::Loading(path.clone());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let load = path.clone();
            let result = cx
                .background_executor()
                .spawn(async move { xd_art_core::io::load(&load) })
                .await;
            this.update(cx, |root, cx| {
                root.doc = match result {
                    Ok(image) => {
                        root.marks.image.get_or_insert_with(Instant::now);
                        Doc::Ready {
                            path,
                            image: Arc::new(image),
                        }
                    }
                    Err(e) => Doc::Failed(e.to_string()),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Shows or hides a side panel.
    pub fn toggle(&mut self, side: Side, cx: &mut Context<Self>) {
        match side {
            Side::Left => self.layout.left_open = !self.layout.left_open,
            Side::Right => self.layout.right_open = !self.layout.right_open,
        }
        cx.notify();
    }

    fn on_move(&mut self, e: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(side) = self.resizing else { return };
        if !e.dragging() {
            self.resizing = None;
            return;
        }
        let x = f32::from(e.position.x);
        let total = f32::from(window.viewport_size().width);
        match side {
            Side::Left => self.layout.left_width = x.clamp(PANEL_MIN, PANEL_MAX),
            Side::Right => self.layout.right_width = (total - x).clamp(PANEL_MIN, PANEL_MAX),
        }
        cx.notify();
    }

    fn title(&self) -> SharedString {
        match &self.doc {
            Doc::Ready { path, .. } | Doc::Loading(path) => path
                .file_name()
                .map_or_else(
                    || path.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                )
                .into(),
            _ => "xd-art".into(),
        }
    }

    fn top_bar(&self, t: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let button = |id: &'static str, name: &'static str, on: bool| {
            div()
                .id(id)
                .size(px(26.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.0))
                .when(on, |d| d.bg(t.elevated))
                .hover(|d| d.bg(t.elevated))
                .child(svg().path(icon(name)).size(px(14.0)).text_color(if on {
                    t.text
                } else {
                    t.muted
                }))
        };
        div()
            .h(px(36.0))
            .flex_none()
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .bg(t.panel)
            .border_b_1()
            .border_color(t.border)
            .child(
                button("toggle-left", "chevron-right", self.layout.left_open)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle(Side::Left, cx))),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .justify_center()
                    .text_color(t.muted)
                    .child(self.title()),
            )
            .child(
                button("toggle-right", "chevron-down", self.layout.right_open)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle(Side::Right, cx))),
            )
    }

    fn panel(&self, side: Side, t: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let width = match side {
            Side::Left => self.layout.left_width,
            Side::Right => self.layout.right_width,
        };
        let handle = div()
            .id(match side {
                Side::Left => "resize-left",
                Side::Right => "resize-right",
            })
            .w(px(5.0))
            .h_full()
            .flex_none()
            .cursor_col_resize()
            .hover(|d| d.bg(t.border))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.resizing = Some(side);
                    cx.notify();
                }),
            );
        let body = div()
            .w(px(width))
            .h_full()
            .flex()
            .flex_col()
            .bg(t.panel)
            .p_3()
            .gap_2()
            .children(match side {
                Side::Left => self.left_contents(t),
                Side::Right => self.right_contents(t),
            });
        let row = div().h_full().flex().flex_none();
        match side {
            Side::Left => row
                .border_r_1()
                .border_color(t.border)
                .child(body)
                .child(handle),
            Side::Right => row
                .border_l_1()
                .border_color(t.border)
                .child(handle)
                .child(body),
        }
    }

    fn left_contents(&self, t: Palette) -> Vec<AnyElement> {
        [
            "Input",
            "Prepare",
            "Sampling",
            "Color",
            "Dithering",
            "Post",
            "Output",
        ]
        .into_iter()
        .map(|name| {
            div()
                .flex()
                .items_center()
                .gap_1()
                .text_size(theme::size::title())
                .text_color(t.text)
                .child(
                    svg()
                        .path(icon("chevron-right"))
                        .size(px(12.0))
                        .text_color(t.muted),
                )
                .child(name)
                .into_any_element()
        })
        .collect()
    }

    fn right_contents(&self, t: Palette) -> Vec<AnyElement> {
        let row = |k: &str, v: String| {
            div()
                .flex()
                .justify_between()
                .text_size(theme::size::body())
                .child(div().text_color(t.muted).child(k.to_string()))
                .child(div().font(theme::number_font()).text_color(t.text).child(v))
                .into_any_element()
        };
        match &self.doc {
            Doc::Ready { image, .. } => vec![
                row("source", format!("{} x {}", image.width(), image.height())),
                row(
                    "alpha",
                    if image.alpha().is_some() {
                        "yes".into()
                    } else {
                        "no".into()
                    },
                ),
            ],
            _ => vec![row("source", "none".into())],
        }
    }

    fn preview(&self, t: Palette) -> impl IntoElement {
        let text: SharedString = match &self.doc {
            Doc::Empty => "drop an image here".into(),
            Doc::Loading(_) => format!("opening {}", self.title()).into(),
            Doc::Ready { image, .. } => {
                format!("{}: {}x{}", self.title(), image.width(), image.height()).into()
            }
            Doc::Failed(e) => e.clone().into(),
        };
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .p_4()
            .bg(t.base)
            .text_size(theme::size::large())
            .text_color(t.muted)
            .child(svg().path(icon("open")).size_8().text_color(t.border))
            .child(div().max_w_full().text_center().child(text))
    }

    fn status_bar(&self, t: Palette) -> impl IntoElement {
        let text = match &self.doc {
            Doc::Ready { image, .. } => format!("{} x {}", image.width(), image.height()),
            Doc::Loading(_) => "loading".into(),
            Doc::Failed(_) => "error".into(),
            Doc::Empty => String::new(),
        };
        div()
            .h(px(22.0))
            .flex_none()
            .flex()
            .items_center()
            .px_3()
            .bg(t.panel)
            .border_t_1()
            .border_color(t.border)
            .text_size(theme::size::small())
            .text_color(t.muted)
            .font(theme::number_font())
            .child(text)
    }

    fn footer(t: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .h(px(20.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .bg(t.base)
            .child(
                div()
                    .id("footer-link")
                    .cursor_pointer()
                    .text_size(theme::size::small())
                    .text_color(t.muted)
                    .hover(|d| d.text_color(t.accent))
                    .on_click(cx.listener(|_, _, _, cx| cx.open_url(FOOTER_URL)))
                    .child("github/joaopssx"),
            )
    }
}

/// Width taken by the open panels, for layout math.
pub fn panels_width(l: &Layout) -> Pixels {
    let mut w = 0.0;
    if l.left_open {
        w += l.left_width;
    }
    if l.right_open {
        w += l.right_width;
    }
    px(w)
}

impl Render for Root {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx).c;
        let mut middle = div().flex_1().flex().min_h_0();
        if self.layout.left_open {
            middle = middle.child(self.panel(Side::Left, t, cx));
        }
        middle = middle.child(self.preview(t));
        if self.layout.right_open {
            middle = middle.child(self.panel(Side::Right, t, cx));
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(t.base)
            .font(theme::ui_font())
            .text_size(theme::size::body())
            .text_color(t.text)
            .on_mouse_move(cx.listener(Self::on_move))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.resizing.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .child(self.top_bar(t, cx))
            .child(middle)
            .child(self.status_bar(t))
            .child(Self::footer(t, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_width_counts_open_panels_only() {
        let mut l = Layout::default();
        assert_eq!(panels_width(&l), px(540.0));
        l.right_open = false;
        assert_eq!(panels_width(&l), px(290.0));
    }
}
