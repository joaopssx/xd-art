//! `xd-art`: the desktop interface.

use gpui::{
    App, Application, Bounds, Context, TitlebarOptions, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, rgb, size,
};

struct Root;

impl Render for Root {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().bg(rgb(0x000e_0f14))
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1280.0), px(800.0)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("xd-art".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        if let Err(err) = cx.open_window(options, |_, cx| cx.new(|_| Root)) {
            eprintln!("could not open window: {err}");
            cx.quit();
        }
    });
}
