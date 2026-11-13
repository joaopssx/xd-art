//! The root view.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use gpui::{Context, Window, div, prelude::*, rgb};
use xd_art_core::buffer::Image;

use crate::Marks;

/// What the window is showing.
pub enum Doc {
    /// Nothing open yet.
    Empty,
    /// A file is being decoded.
    Loading(PathBuf),
    /// A decoded image.
    Ready { path: PathBuf, image: Arc<Image> },
    /// Opening failed.
    Failed(String),
}

/// The application window's content.
pub struct Root {
    marks: Marks,
    doc: Doc,
}

impl Root {
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
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
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
            .items_center()
            .justify_center()
            .bg(rgb(0x000e_0f14))
            .text_color(rgb(0x008b_91a5))
            .child(text)
    }
}
