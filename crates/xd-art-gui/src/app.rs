//! The root view: top bar, side panels around the preview, status bar and footer.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use std::time::Duration;

use gpui::{
    AnyElement, Context, ExternalPaths, FocusHandle, MouseButton, MouseMoveEvent,
    PathPromptOptions, Pixels, PromptLevel, SharedString, Window, div, prelude::*, px, svg,
};
use xd_art_core::buffer::Image;
use xd_art_core::settings::{PaletteSource, Settings};

use crate::Marks;
use crate::actions::{self, Open, ToggleLeft, ToggleRight};
use crate::assets::icon;
use crate::config::Config;
use crate::theme::{self, Palette, Theme};

/// What a dropped or opened file is, by its extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Something to convert.
    Image,
    /// Colors to use.
    Palette,
    /// Settings to apply.
    Recipe,
    /// Not something xd-art opens.
    Other,
}

/// Sorts a path by extension.
pub fn kind(path: &std::path::Path) -> Kind {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "bmp" | "tga" | "qoi" | "gif" => Kind::Image,
        "gpl" | "hex" | "pal" => Kind::Palette,
        "json" => Kind::Recipe,
        _ => Kind::Other,
    }
}

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
    focus: FocusHandle,
    config: Config,
    settings: Settings,
    /// One line for the status bar: what just happened.
    notice: Option<SharedString>,
    /// Bumped on every config change; a pending write only goes ahead if it is still
    /// the latest.
    save_generation: u64,
}

impl Root {
    /// An empty view with the preferences from `config`; `marks` carries the startup
    /// times so far. The last settings come back from the config, so an image opened
    /// from the command line is converted the way the previous one was.
    pub fn new(
        mut marks: Marks,
        config: Config,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        marks.window = Some(Instant::now());
        let focus = cx.focus_handle();
        window.focus(&focus);
        let layout = Layout {
            left_open: config.left_open,
            right_open: config.right_open,
            left_width: config.left_width,
            right_width: config.right_width,
        };
        let settings = xd_art_core::recipe::from_json(&config.recipe).unwrap_or_default();
        Self {
            marks,
            doc: Doc::Empty,
            layout,
            resizing: None,
            focus,
            config,
            settings,
            notice: None,
            save_generation: 0,
        }
    }

    /// Writes the config half a second after the last change, on a background thread.
    fn persist(&mut self, cx: &mut Context<Self>) {
        self.config.left_open = self.layout.left_open;
        self.config.right_open = self.layout.right_open;
        self.config.left_width = self.layout.left_width;
        self.config.right_width = self.layout.right_width;
        self.config.theme = Theme::get(cx).variant.index();
        self.config.recipe = xd_art_core::recipe::to_json(&self.settings);
        self.save_generation += 1;
        let generation = self.save_generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(500))
                .await;
            let Ok(Some(cfg)) = this.read_with(cx, |root, _| {
                (root.save_generation == generation).then(|| root.config.clone())
            }) else {
                return;
            };
            cx.background_executor()
                .spawn(async move { crate::config::store(&cfg) })
                .await
                .ok();
        })
        .detach();
    }

    /// Opens what was given: images one by one or as a batch, palettes and recipes as
    /// settings.
    pub fn open_many(&mut self, paths: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let images: Vec<PathBuf> = paths
            .iter()
            .filter(|p| kind(p) == Kind::Image)
            .cloned()
            .collect();
        for p in &paths {
            match kind(p) {
                Kind::Palette => self.apply_palette(p, cx),
                Kind::Recipe => self.apply_recipe(p, cx),
                Kind::Other => {
                    self.notice =
                        Some(format!("{} is not an image, palette or recipe", p.display()).into())
                }
                Kind::Image => {}
            }
        }
        match images.len() {
            0 => cx.notify(),
            1 => self.open(images[0].clone(), cx),
            n => self.ask_batch(images, n, window, cx),
        }
    }

    fn apply_palette(&mut self, path: &std::path::Path, cx: &mut Context<Self>) {
        match xd_art_core::palette::Palette::load(path) {
            Ok(p) => {
                self.notice = Some(format!("palette {} ({} colors)", p.name, p.len()).into());
                self.settings.colors.palette = PaletteSource::Colors(p.colors().to_vec());
                self.persist(cx);
            }
            Err(e) => self.notice = Some(e.to_string().into()),
        }
    }

    fn apply_recipe(&mut self, path: &std::path::Path, cx: &mut Context<Self>) {
        let read = std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|t| {
                xd_art_core::recipe::from_json(&t)
                    .or_else(|_| xd_art_core::recipe::from_original(&t))
                    .map_err(|e| e.to_string())
            });
        match read {
            Ok(s) => {
                self.settings = s;
                self.notice = Some(format!("recipe {}", path.display()).into());
                self.persist(cx);
            }
            Err(e) => self.notice = Some(e.into()),
        }
    }

    /// Several images at once: offer to convert them all with the current settings.
    fn ask_batch(
        &mut self,
        images: Vec<PathBuf>,
        n: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let answer = window.prompt(
            PromptLevel::Info,
            &format!("Convert {n} images with the current settings?"),
            Some("Pick a folder for the results next."),
            &["Convert", "Open the first one", "Cancel"],
            cx,
        );
        let settings = self.settings.clone();
        cx.spawn(async move |this, cx| {
            match answer.await {
                Ok(0) => {}
                Ok(1) => {
                    this.update(cx, |root, cx| root.open(images[0].clone(), cx))
                        .ok();
                    return;
                }
                _ => return,
            }
            let pick = cx.update(|cx| {
                cx.prompt_for_paths(PathPromptOptions {
                    files: false,
                    directories: true,
                    multiple: false,
                    prompt: None,
                })
            });
            let Ok(pick) = pick else { return };
            let Ok(Ok(Some(dirs))) = pick.await else {
                return;
            };
            let Some(dir) = dirs.into_iter().next() else {
                return;
            };
            let done = cx
                .background_executor()
                .spawn(async move { batch(&images, &dir, &settings) })
                .await;
            this.update(cx, |root, cx| {
                root.notice = Some(done.into());
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Asks for files to open.
    fn prompt_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pick = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = pick.await else {
                return;
            };
            handle
                .update(cx, |_, window, cx| {
                    this.update(cx, |root, cx| root.open_many(paths, window, cx))
                        .ok();
                })
                .ok();
        })
        .detach();
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
                        root.config.remember(&path);
                        root.persist(cx);
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
        self.persist(cx);
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
                button("open", "open", false)
                    .on_click(cx.listener(|this, _, window, cx| this.prompt_open(window, cx))),
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

    fn preview(&self, t: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let text: SharedString = match &self.doc {
            Doc::Empty => "drop an image here".into(),
            Doc::Loading(_) => format!("opening {}", self.title()).into(),
            Doc::Ready { image, .. } => {
                format!("{}: {}x{}", self.title(), image.width(), image.height()).into()
            }
            Doc::Failed(e) => e.clone().into(),
        };
        let recents: Vec<AnyElement> = if matches!(self.doc, Doc::Empty) {
            self.config
                .recents
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let path = p.clone();
                    div()
                        .id(("recent", i))
                        .px_2()
                        .py_1()
                        .rounded(px(4.0))
                        .text_size(theme::size::body())
                        .text_color(t.muted)
                        .cursor_pointer()
                        .hover(|d| d.bg(t.elevated).text_color(t.text))
                        .on_click(cx.listener(move |this, _, _, cx| this.open(path.clone(), cx)))
                        .child(p.display().to_string())
                        .into_any_element()
                })
                .collect()
        } else {
            Vec::new()
        };
        div()
            .id("preview")
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
            .when(!recents.is_empty(), |d| {
                d.child(div().mt_4().text_size(theme::size::small()).child("recent"))
                    .child(div().flex().flex_col().items_center().children(recents))
            })
    }

    fn status_bar(&self, t: Palette) -> impl IntoElement {
        let mut text = match &self.doc {
            Doc::Ready { image, .. } => format!("{} x {}", image.width(), image.height()),
            Doc::Loading(_) => "loading".into(),
            Doc::Failed(_) => "error".into(),
            Doc::Empty => String::new(),
        };
        if let Some(n) = &self.notice {
            if !text.is_empty() {
                text.push_str("   ");
            }
            text.push_str(n);
        }
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

/// Converts `images` into `dir` with `settings`, as PNG with the same names. Returns a
/// one line summary.
fn batch(images: &[PathBuf], dir: &std::path::Path, settings: &Settings) -> String {
    let mut p = xd_art_core::pipeline::Pipeline::default();
    let (mut ok, mut failed) = (0, 0);
    for (i, path) in images.iter().enumerate() {
        let out = dir.join(format!(
            "{}.png",
            path.file_stem().and_then(|s| s.to_str()).unwrap_or("image")
        ));
        let done = xd_art_core::io::load(path).and_then(|img| {
            p.run(
                &img,
                i as u64 + 1,
                settings,
                &xd_art_core::cancel::Cancel::new(),
            )?;
            let img = p.output().ok_or(xd_art_core::Error::Cancelled)?;
            xd_art_core::io::save(img, &out)
        });
        if done.is_ok() { ok += 1 } else { failed += 1 }
    }
    format!("{ok} converted into {}, {failed} failed", dir.display())
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
        middle = middle.child(self.preview(t, cx));
        if self.layout.right_open {
            middle = middle.child(self.panel(Side::Right, t, cx));
        }
        div()
            .id("root")
            .track_focus(&self.focus)
            .key_context(actions::CONTEXT)
            .on_action(cx.listener(|this, _: &Open, window, cx| this.prompt_open(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleLeft, _, cx| this.toggle(Side::Left, cx)))
            .on_action(cx.listener(|this, _: &ToggleRight, _, cx| this.toggle(Side::Right, cx)))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.open_many(paths.paths().to_vec(), window, cx);
            }))
            .drag_over::<ExternalPaths>(move |style, _, _, _| {
                style.border_2().border_color(t.accent)
            })
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
                        this.persist(cx);
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
    fn kinds_by_extension() {
        use std::path::Path;
        assert_eq!(kind(Path::new("a.PNG")), Kind::Image);
        assert_eq!(kind(Path::new("a.gpl")), Kind::Palette);
        assert_eq!(kind(Path::new("r.json")), Kind::Recipe);
        assert_eq!(kind(Path::new("notes.txt")), Kind::Other);
    }

    #[test]
    fn panel_width_counts_open_panels_only() {
        let mut l = Layout::default();
        assert_eq!(panels_width(&l), px(540.0));
        l.right_open = false;
        assert_eq!(panels_width(&l), px(290.0));
    }
}
