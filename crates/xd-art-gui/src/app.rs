//! The root view.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use gpui::{Context, Window, div, prelude::*, svg};
use xd_art_core::buffer::Image;

use crate::Marks;
use crate::assets::icon;
use crate::theme::{self, Theme};

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

/// The application window's content.
pub struct Root {
    marks: Marks,
    doc: Doc,
}

impl Root {
    /// An empty view; `marks` carries the startup times so far.
    pub fn new(mut marks: Marks) -> Self {
        marks.window = Some(Instant::now());
        Self {
            marks,
            doc: Doc::Empty,
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
}

impl Render for Root {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx).c;
        let text = match &self.doc {
            Doc::Empty => "drop an image here".to_string(),
            Doc::Loading(p) => format!("opening {}", p.display()),
            Doc::Ready { path, image } => {
                format!("{}: {}x{}", path.display(), image.width(), image.height())
            }
            Doc::Failed(e) => e.clone(),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .bg(t.base)
            .font(theme::ui_font())
            .text_size(theme::size::large())
            .text_color(t.muted)
            .child(svg().path(icon("open")).size_8().text_color(t.border))
            .child(text)
    }
}
