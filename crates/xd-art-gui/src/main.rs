//! `xd-art`: the desktop interface.
//!
//! Startup does as little as possible before the window is on screen: no image decode,
//! no palettes, no config parsing beyond one small file, no network, no Lua. Files given
//! on the command line (or dropped on the executable, which is the same thing) are
//! decoded on a background thread once the window exists. `docs/performance.md` lists
//! every step.

mod app;

use std::path::PathBuf;
use std::time::Instant;

use gpui::{
    App, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, prelude::*, px, size,
};

/// Points in time during startup, kept for the trace and the status bar.
#[derive(Clone, Copy, Debug)]
pub struct Marks {
    /// When `main` started.
    pub start: Instant,
    /// When the window was created.
    pub window: Option<Instant>,
    /// When the first frame was presented.
    pub first_frame: Option<Instant>,
    /// When the first image was ready.
    pub image: Option<Instant>,
}

fn main() {
    let start = Instant::now();
    // Paths only; nothing is opened until the window is up.
    let paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1280.0), px(800.0)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("xd-art".into()),
                ..Default::default()
            }),
            window_min_size: Some(size(px(640.0), px(420.0))),
            ..Default::default()
        };
        let marks = Marks {
            start,
            window: None,
            first_frame: None,
            image: None,
        };
        let opened = cx.open_window(options, move |window, cx| {
            let root = cx.new(|_| app::Root::new(marks));
            let handle = root.clone();
            window.on_next_frame(move |_, cx| handle.update(cx, |root, _| root.first_frame()));
            // The decode runs on a background thread, so starting it now cannot delay
            // the window; it only has to wait for the window to exist.
            if let Some(first) = paths.first().cloned() {
                root.update(cx, |root, cx| root.open(first, cx));
            }
            root
        });
        if let Err(err) = opened {
            eprintln!("could not open window: {err}");
            cx.quit();
        }
    });
}
