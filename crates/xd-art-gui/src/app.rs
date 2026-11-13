//! The root view: top bar, side panels around the preview, status bar and footer.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use std::sync::Mutex;
use std::time::Duration;

use gpui::{
    Animation, AnimationExt, AnyElement, Context, Entity, ExternalPaths, FocusHandle, MouseButton,
    MouseDownEvent, MouseMoveEvent, PathPromptOptions, Pixels, PromptLevel, ScrollDelta,
    ScrollWheelEvent, SharedString, Subscription, Window, div, ease_out_quint, prelude::*, px, svg,
};
use xd_art_core::buffer::Image;
use xd_art_core::cancel::Cancel;
use xd_art_core::pipeline::{Pipeline, Report, Stage};
use xd_art_core::settings::{DitherMode, PaletteSource, SampleMode, Settings, Size};

use crate::Marks;
use crate::actions::{
    self, AccentFromImage, ActualSize, CommandPalette, Compare, Fit, Fullscreen, Gallery,
    KeepVariation, NextTheme, Open, Redo, ReduceMotion, ShortcutSheet, ToggleGrid, ToggleLeft,
    ToggleRight, Undo, Variations, ZoomIn, ZoomOut,
};
use crate::assets::icon;
use crate::config::Config;
use crate::gallery;
use crate::history::History;
use crate::panels::{Choice, Section, Toggle};
use crate::preview::Viewer;
use crate::strip::Strip;
use crate::theme::{self, Palette, Theme};
use crate::widgets::{Slider, SliderEvent, SliderSpec};

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

/// A slider bound to one number in the settings.
pub struct Control {
    /// The slider.
    pub slider: Entity<Slider>,
    /// Writes the slider's value into the settings.
    pub apply: fn(&mut Settings, f32),
    /// Reads the value from the settings.
    pub read: fn(&Settings) -> f32,
    /// Section of the settings panel it is in.
    pub section: Section,
    /// Whether it has an effect with the current settings.
    pub visible: fn(&Settings) -> bool,
    _sub: Subscription,
}

/// The application window's content.
pub struct Root {
    pub(crate) marks: Marks,
    pub(crate) doc: Doc,
    pub(crate) layout: Layout,
    /// The panel edge being dragged, if any.
    pub(crate) resizing: Option<Side>,
    pub(crate) focus: FocusHandle,
    pub(crate) config: Config,
    pub(crate) settings: Settings,
    /// One line for the status bar: what just happened.
    pub(crate) notice: Option<SharedString>,
    /// Bumped on every config change; a pending write only goes ahead if it is still
    /// the latest.
    pub(crate) save_generation: u64,
    pub(crate) viewer: Viewer,
    pub(crate) controls: Vec<Control>,
    pub(crate) choices: Vec<Choice>,
    pub(crate) toggles: Vec<Toggle>,
    /// Collapsed sections of the settings panel, indexed like [`Section::ALL`].
    pub(crate) collapsed: [bool; 7],
    /// The choice whose option list is open.
    pub(crate) open_choice: Option<&'static str>,
    /// The converted image currently shown.
    pub(crate) output: Option<Arc<Image>>,
    /// The source at reduced size, for big images.
    pub(crate) proxy: Option<Arc<Image>>,
    /// Identifies the open image in the pipeline's cache.
    pub(crate) source_id: u64,
    /// Bumped on every conversion request; results of older requests are dropped.
    pub(crate) generation: u64,
    /// The token of the running conversion.
    pub(crate) cancel: Option<Cancel>,
    /// The pipeline, used by one background conversion at a time.
    pub(crate) pipeline: Arc<Mutex<Pipeline>>,
    /// How long the last shown conversion took, from request to result, and what ran.
    pub(crate) last_run: Option<(Duration, Report)>,
    /// The image shown came from the reduced copy or a cheaper metric.
    pub(crate) approximate: bool,
    /// The palette strip.
    pub(crate) strip: Strip,
    /// The dither gallery and hover previews.
    pub(crate) gallery: gallery::Gallery,
    /// Undo and redo.
    pub(crate) history: History,
    /// Settings are being restored from the history and must not be recorded.
    pub(crate) restoring: bool,
    /// Versions side by side.
    pub(crate) variations: crate::variations::Variations,
    /// Command palette and shortcut sheet.
    pub(crate) commands: crate::commands::Commands,
    /// Image only mode.
    pub(crate) fullscreen: bool,
    /// The mouse is at the top edge in image only mode, so the top bar shows.
    pub(crate) edge_hover: bool,
    /// Bumped every time a panel opens, so its fade in starts over.
    pub(crate) panel_opened: u32,
    /// Frames of an animated GIF; empty for still images.
    pub(crate) frames: crate::frames::Frames,
    /// Pencil, eraser and mask brushes.
    pub(crate) retouch: crate::retouch::Retouch,
    /// Export scale `output` was made with.
    pub(crate) output_scale: u32,
}

/// Length of interface transitions.
const TRANSITION: Duration = Duration::from_millis(120);

/// Wait after the last change before the exact conversion starts, so a burst of
/// changes (typing a value, clicking through options) runs it once.
const SETTLE: Duration = Duration::from_millis(120);

/// Sources with more pixels than this get a reduced copy that is converted first, so
/// something shows within a few tens of milliseconds.
const PROXY_FROM: usize = 1_500_000;
/// Longest side of the reduced copy.
const PROXY_SIDE: u32 = 1024;

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
            viewer: Viewer::default(),
            controls: Vec::new(),
            choices: crate::panels::choices(),
            toggles: crate::panels::toggles(),
            collapsed: [false; 7],
            open_choice: None,
            output: None,
            proxy: None,
            source_id: 0,
            generation: 0,
            cancel: None,
            pipeline: Arc::new(Mutex::new(Pipeline::default())),
            last_run: None,
            approximate: false,
            strip: Strip::default(),
            gallery: gallery::Gallery::default(),
            history: History::default(),
            restoring: false,
            variations: crate::variations::Variations::default(),
            commands: crate::commands::Commands::default(),
            fullscreen: false,
            edge_hover: false,
            panel_opened: 0,
            frames: crate::frames::Frames::default(),
            retouch: crate::retouch::Retouch::default(),
            output_scale: 1,
        }
    }

    /// Adds a slider for one setting.
    fn control(
        &mut self,
        (section, spec): (Section, SliderSpec),
        read: fn(&Settings) -> f32,
        apply: fn(&mut Settings, f32),
        visible: fn(&Settings) -> bool,
        cx: &mut Context<Self>,
    ) {
        let start = read(&self.settings);
        let slider = cx.new(|cx| Slider::new(spec, start, cx));
        let sub = cx.subscribe(&slider, move |this, _, e: &SliderEvent, cx| {
            let SliderEvent::Changed { value, live } = *e;
            apply(&mut this.settings, value);
            this.settings_changed(live, cx);
        });
        self.controls.push(Control {
            slider,
            apply,
            read,
            section,
            visible,
            _sub: sub,
        });
    }

    /// Builds the sliders. Kept apart from `new` because creating entities needs the
    /// view's own context.
    #[allow(clippy::too_many_lines)] // one line per slider, the list is the point
    pub fn build_controls(&mut self, cx: &mut Context<Self>) {
        use Section::{Color, Dithering, Output, Post, Prepare, Sampling};
        let spec = |section: Section,
                    label: &str,
                    (min, max, step): (f32, f32, f32),
                    default: f32,
                    decimals: usize| {
            (
                section,
                SliderSpec {
                    label: label.to_string().into(),
                    min,
                    max,
                    step,
                    default,
                    unit: "",
                    decimals,
                },
            )
        };
        let always = |_: &Settings| true;
        let absolute = |s: &Settings| matches!(s.sample.size, Size::Absolute { .. });
        let relative = |s: &Settings| matches!(s.sample.size, Size::Relative { .. });
        self.control(
            spec(Prepare, "blur", (0.0, 32.0, 0.25), 0.0, 2),
            |s| s.blur.radius,
            |s, v| s.blur.radius = v,
            always,
            cx,
        );
        self.control(
            spec(Prepare, "box blur from", (1.0, 32.0, 0.5), 5.0, 1),
            |s| s.blur.box_from,
            |s, v| s.blur.box_from = v,
            |s| s.blur.radius > 0.0 && !s.blur.box_approx,
            cx,
        );
        self.control(
            spec(Sampling, "width", (1.0, 512.0, 1.0), 64.0, 0),
            |s| match s.sample.size {
                Size::Absolute { width, .. } => width as f32,
                Size::Relative { .. } => 64.0,
            },
            |s, v| {
                if let Size::Absolute { width, .. } = &mut s.sample.size {
                    *width = v as u32;
                }
            },
            absolute,
            cx,
        );
        self.control(
            spec(Sampling, "height", (1.0, 512.0, 1.0), 64.0, 0),
            |s| match s.sample.size {
                Size::Absolute { height, .. } => height as f32,
                Size::Relative { .. } => 64.0,
            },
            |s, v| {
                if let Size::Absolute { height, .. } = &mut s.sample.size {
                    *height = v as u32;
                }
            },
            |s| {
                matches!(
                    s.sample.size,
                    Size::Absolute {
                        keep_aspect: false,
                        ..
                    }
                )
            },
            cx,
        );
        self.control(
            spec(Sampling, "divide by", (1.0, 32.0, 1.0), 4.0, 0),
            |s| match s.sample.size {
                Size::Relative { x, .. } => x as f32,
                Size::Absolute { .. } => 4.0,
            },
            |s, v| {
                if let Size::Relative { x, y } = &mut s.sample.size {
                    *x = v as u32;
                    *y = v as u32;
                }
            },
            relative,
            cx,
        );
        self.control(
            spec(Sampling, "offset x", (-1.0, 1.0, 0.01), 0.0, 2),
            |s| s.sample.offset_x,
            |s, v| s.sample.offset_x = v,
            always,
            cx,
        );
        self.control(
            spec(Sampling, "offset y", (-1.0, 1.0, 0.01), 0.0, 2),
            |s| s.sample.offset_y,
            |s, v| s.sample.offset_y = v,
            always,
            cx,
        );
        self.control(
            spec(Sampling, "edge strength", (0.0, 1.0, 0.01), 0.6, 2),
            |s| s.sample.edge_strength,
            |s, v| s.sample.edge_strength = v,
            |s| s.sample.mode == SampleMode::Edge,
            cx,
        );
        self.control(
            spec(Color, "sharpen", (0.0, 4.0, 0.05), 0.0, 2),
            |s| s.adjust.sharpen,
            |s, v| s.adjust.sharpen = v,
            always,
            cx,
        );
        self.control(
            spec(Color, "brightness", (-1.0, 1.0, 0.01), 0.0, 2),
            |s| s.adjust.brightness,
            |s, v| s.adjust.brightness = v,
            always,
            cx,
        );
        self.control(
            spec(Color, "contrast", (0.0, 3.0, 0.01), 1.0, 2),
            |s| s.adjust.contrast,
            |s, v| s.adjust.contrast = v,
            always,
            cx,
        );
        self.control(
            spec(Color, "saturation", (0.0, 3.0, 0.01), 1.0, 2),
            |s| s.adjust.saturation,
            |s, v| s.adjust.saturation = v,
            always,
            cx,
        );
        self.control(
            spec(Color, "hue", (-180.0, 180.0, 1.0), 0.0, 0),
            |s| s.adjust.hue,
            |s, v| s.adjust.hue = v,
            always,
            cx,
        );
        self.control(
            spec(Color, "gamma", (0.1, 4.0, 0.01), 1.0, 2),
            |s| s.adjust.gamma,
            |s, v| s.adjust.gamma = v,
            always,
            cx,
        );
        self.control(
            spec(Color, "tint strength", (0.0, 1.0, 0.01), 1.0, 2),
            |s| s.adjust.tint_strength,
            |s, v| s.adjust.tint_strength = v,
            |s| s.adjust.tint != [255; 3],
            cx,
        );
        self.control(
            spec(Color, "colors", (2.0, 64.0, 1.0), 16.0, 0),
            |s| match &s.colors.palette {
                PaletteSource::Extract { count, .. } => f32::from(*count),
                _ => 16.0,
            },
            |s, v| {
                if let PaletteSource::Extract { count, .. } = &mut s.colors.palette {
                    *count = v as u16;
                }
            },
            |s| matches!(s.colors.palette, PaletteSource::Extract { .. }),
            cx,
        );
        self.control(
            spec(Dithering, "amount", (0.0, 1.0, 0.01), 0.2, 2),
            |s| s.dither.amount,
            |s, v| s.dither.amount = v,
            |s| s.dither.mode.is_ordered(),
            cx,
        );
        self.control(
            spec(Dithering, "alpha threshold", (0.0, 255.0, 1.0), 128.0, 0),
            |s| f32::from(s.dither.alpha_threshold),
            |s, v| s.dither.alpha_threshold = v as u8,
            always,
            cx,
        );
        self.control(
            spec(Dithering, "target colors", (2.0, 64.0, 1.0), 8.0, 0),
            |s| f32::from(s.dither.target_colors),
            |s, v| s.dither.target_colors = v as u16,
            |s| s.dither.mode == DitherMode::MedianCut,
            cx,
        );
        self.control(
            spec(Post, "merge colors closer than", (0.0, 64.0, 1.0), 0.0, 0),
            |s| f32::from(s.post.merge),
            |s, v| s.post.merge = v as u8,
            always,
            cx,
        );
        self.control(
            spec(Output, "scale", (1.0, 16.0, 1.0), 1.0, 0),
            |s| s.output.scale as f32,
            |s, v| s.output.scale = v as u32,
            always,
            cx,
        );
    }

    /// Converts the open image with the current settings on a background thread. A
    /// new request cancels the one running; a result that is no longer the latest
    /// request is never shown. With `quick`, or for a big image the first time, the
    /// reduced copy is converted first.
    pub(crate) fn convert(&mut self, quick: bool, cx: &mut Context<Self>) {
        let Doc::Ready { image, .. } = &self.doc else {
            return;
        };
        let full = image.clone();
        if let Some(c) = self.cancel.take() {
            c.cancel();
        }
        let cancel = Cancel::new();
        self.cancel = Some(cancel.clone());
        self.generation += 1;
        let generation = self.generation;
        let mut settings = self.settings.clone();
        if quick {
            // The LAB metrics cost several times more per pixel; redmean stands in
            // while dragging and the exact metric runs on release.
            settings.colors.metric = settings.colors.metric.preview();
        }
        let exact_metric = settings.colors.metric == self.settings.colors.metric;
        let pipeline = self.pipeline.clone();
        let id = self.source_id;
        // Proxy results use their own id, so they do not evict the full size ones.
        let proxy = self
            .proxy
            .clone()
            .filter(|_| quick || self.output.is_none());
        let started = Instant::now();
        let scale = settings.output.scale;
        cx.spawn(async move |this, cx| {
            let mut sources = Vec::new();
            if let Some(p) = proxy {
                sources.push((p, id | 1 << 63));
            }
            if !quick || sources.is_empty() {
                sources.push((full, id));
            }
            let last = sources.len() - 1;
            for (step, (src, sid)) in sources.into_iter().enumerate() {
                let (pipeline, settings, cancel) =
                    (pipeline.clone(), settings.clone(), cancel.clone());
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let mut p = pipeline
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        let report = p.run(&src, sid, &settings, &cancel)?;
                        let palette = p.palette().map(<[[u8; 3]]>::to_vec).unwrap_or_default();
                        let mut counts = vec![0u64; palette.len()];
                        if let Some(ix) = p.indexed() {
                            for (i, &c) in ix.index.iter().enumerate() {
                                if !ix.is_clear(i)
                                    && let Some(n) = counts.get_mut(c as usize)
                                {
                                    *n += 1;
                                }
                            }
                        }
                        Ok::<_, xd_art_core::Error>((report, p.output().cloned(), palette, counts))
                    })
                    .await;
                let shown = this.update(cx, |root, cx| {
                    if root.generation != generation {
                        return false;
                    }
                    match result {
                        Ok((report, Some(out), palette, counts)) => {
                            root.output = Some(Arc::new(out));
                            root.output_scale = scale;
                            root.strip.colors = palette;
                            root.strip.counts = counts;
                            if root.config.accent_from_image {
                                root.apply_theme(cx);
                            }
                            root.last_run = Some((started.elapsed(), report));
                            root.approximate = sid != id || !exact_metric || step < last;
                            if !root.approximate
                                && let Some(out) = &root.output
                            {
                                root.history.set_preview(out.clone());
                            }
                            root.notice = None;
                        }
                        Ok((_, None, _, _)) | Err(xd_art_core::Error::Cancelled) => return false,
                        Err(e) => root.notice = Some(e.to_string().into()),
                    }
                    cx.notify();
                    true
                });
                if !matches!(shown, Ok(true)) {
                    return;
                }
            }
        })
        .detach();
    }

    /// Called after any settings change. `live` is true while a slider is being
    /// dragged, when nothing is written to disk yet.
    pub(crate) fn settings_changed(&mut self, live: bool, cx: &mut Context<Self>) {
        if !live {
            self.sync_controls(cx);
            self.persist(cx);
            if !self.restoring {
                self.history.record(&self.settings);
            }
        }
        if live {
            self.convert(true, cx);
        } else {
            self.settle(cx);
            if self.gallery.open {
                self.fill_gallery(cx);
            }
        }
        cx.notify();
    }

    /// Runs the exact conversion once changes stop for [`SETTLE`].
    fn settle(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SETTLE).await;
            this.update(cx, |root, cx| {
                if root.generation == generation {
                    root.convert(false, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Source size rows for the Input section.
    pub(crate) fn input_rows(&self, t: Palette) -> Vec<AnyElement> {
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
                row("size", format!("{} x {}", image.width(), image.height())),
                row(
                    "transparency",
                    if image.alpha().is_some() {
                        "yes".into()
                    } else {
                        "no".into()
                    },
                ),
            ],
            _ => vec![row("size", "no image".into())],
        }
    }

    /// Puts the settings back into every slider, after a recipe or palette changed them.
    fn sync_controls(&mut self, cx: &mut Context<Self>) {
        for c in &self.controls {
            let v = (c.read)(&self.settings);
            c.slider.update(cx, |s, cx| s.set_value(v, cx));
        }
    }

    /// The source to make small previews from: the reduced copy for big images, the
    /// image itself otherwise.
    pub(crate) fn small_source(&self) -> Option<Arc<Image>> {
        match &self.doc {
            Doc::Ready { image, .. } => Some(self.proxy.clone().unwrap_or_else(|| image.clone())),
            _ => None,
        }
    }

    /// The open image at full size.
    pub(crate) fn full_source(&self) -> Option<Arc<Image>> {
        match &self.doc {
            Doc::Ready { image, .. } => Some(image.clone()),
            _ => None,
        }
    }

    /// Applies the theme from the config, with the accent from the current palette when
    /// that is turned on.
    pub(crate) fn apply_theme(&mut self, cx: &mut Context<Self>) {
        let variant = Theme::get(cx).variant;
        let accent = self
            .config
            .accent_from_image
            .then(|| theme::accent_from(&self.strip.colors, &variant.palette()))
            .flatten();
        Theme::set_accent(variant, accent, cx);
        cx.notify();
    }

    /// The root's focus handle, for widgets that type into the root.
    pub(crate) fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
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
                self.settings_changed(false, cx);
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
                self.settings_changed(false, cx);
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
                .spawn(async move { (load_any(&load), Self::load_project(&load)) })
                .await;
            this.update(cx, |root, cx| {
                let (result, (layer, mask)) = result;
                root.doc = match result {
                    Ok((image, frames)) => {
                        // Retouches and the mask belong to one image.
                        root.retouch.reset(layer);
                        root.settings.mask = mask;
                        root.frames = crate::frames::Frames {
                            list: frames,
                            ..Default::default()
                        };
                        root.marks.image.get_or_insert_with(Instant::now);
                        root.config.remember(&path);
                        root.persist(cx);
                        root.viewer.fit();
                        let image = Arc::new(image);
                        root.source_id += 1;
                        root.output = None;
                        root.history.start(&root.settings);
                        root.proxy = (image.width() as usize * image.height() as usize
                            > PROXY_FROM)
                            .then(|| {
                                let side = image.width().max(image.height());
                                xd_art_core::io::reduce(&image, side.div_ceil(PROXY_SIDE))
                                    .ok()
                                    .map(Arc::new)
                            })
                            .flatten();
                        Doc::Ready { path, image }
                    }
                    Err(e) => Doc::Failed(e.to_string()),
                };
                root.convert(false, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn image_size(&self) -> Option<(u32, u32)> {
        match (&self.output, &self.doc) {
            (Some(out), _) => Some((out.width(), out.height())),
            (None, Doc::Ready { image, .. }) => Some((image.width(), image.height())),
            _ => None,
        }
    }

    fn on_wheel(&mut self, e: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.image_size().is_none() {
            return;
        }
        let dy = match e.delta {
            ScrollDelta::Lines(p) => p.y,
            ScrollDelta::Pixels(p) => f32::from(p.y) / 40.0,
        };
        if dy.abs() < f32::EPSILON {
            return;
        }
        let next = crate::preview::step(self.viewer_zoom(), dy > 0.0);
        self.viewer.zoom_at(next, e.position);
        cx.notify();
    }

    fn viewer_zoom(&self) -> f32 {
        self.viewer.zoom
    }

    fn start_pan(&mut self, e: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if e.button == MouseButton::Left
            && !self.viewer.space
            && !self.viewer.near_split(e.position.x)
            && self.frames.list.is_empty()
            && self.tool_down(e.position, cx)
        {
            return;
        }
        if e.button == MouseButton::Left && self.viewer.near_split(e.position.x) {
            self.viewer.dragging_split = true;
            return;
        }
        if e.button == MouseButton::Middle || (e.button == MouseButton::Left && self.viewer.space) {
            self.viewer.panning = Some(e.position);
            cx.notify();
        }
    }

    /// Shows or hides a side panel.
    pub fn toggle(&mut self, side: Side, cx: &mut Context<Self>) {
        match side {
            Side::Left => self.layout.left_open = !self.layout.left_open,
            Side::Right => self.layout.right_open = !self.layout.right_open,
        }
        self.panel_opened = self.panel_opened.wrapping_add(1);
        self.persist(cx);
        cx.notify();
    }

    fn on_move(&mut self, e: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.fullscreen {
            let near = f32::from(e.position.y) < 48.0;
            if near != self.edge_hover {
                self.edge_hover = near;
                cx.notify();
            }
        }
        if self.retouch.drawing() {
            if e.pressed_button == Some(MouseButton::Left) {
                self.tool_move(e.position, cx);
            } else {
                self.tool_up(cx);
            }
            return;
        }
        if self.viewer.dragging_split {
            if e.pressed_button.is_none() {
                self.viewer.dragging_split = false;
            } else {
                self.viewer.split_at(e.position.x);
                cx.notify();
            }
            return;
        }
        if let Some(last) = self.viewer.panning {
            if e.pressed_button.is_none() {
                self.viewer.panning = None;
            } else {
                self.viewer.pan_by(e.position - last);
                self.viewer.panning = Some(e.position);
                cx.notify();
            }
            return;
        }
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
                button("compare", "compare", self.viewer.compare.is_some()).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.viewer.cycle_compare();
                        cx.notify();
                    },
                )),
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
            .id(match side {
                Side::Left => "left-body",
                Side::Right => "right-body",
            })
            .overflow_y_scroll()
            .children(match side {
                Side::Left => self.settings_panel(t, cx),
                Side::Right => {
                    let mut v = self.right_contents(t);
                    if self.frames.list.is_empty() {
                        v.extend(self.retouch_panel(t, cx));
                    }
                    if self.history.entries.len() > 1 {
                        v.push(
                            div()
                                .mt_3()
                                .text_size(theme::size::title())
                                .text_color(t.text)
                                .child("History")
                                .into_any_element(),
                        );
                        v.extend(self.timeline(t, cx));
                    }
                    v
                }
            });
        let row = div().h_full().flex().flex_none();
        let row = match side {
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
        };
        let id = (
            if side == Side::Left {
                "fade-left"
            } else {
                "fade-right"
            },
            self.panel_opened as usize,
        );
        let animation = Animation::new(if self.config.reduced_motion {
            Duration::ZERO
        } else {
            TRANSITION
        })
        .with_easing(ease_out_quint());
        row.with_animation(id, animation, |el, d| el.opacity(d))
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
        let base = div()
            .id("preview")
            .flex_1()
            .min_w_0()
            .min_h_0()
            .w_full()
            .overflow_hidden()
            .bg(t.base)
            .on_scroll_wheel(cx.listener(Self::on_wheel))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::start_pan))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::start_pan))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &gpui::MouseUpEvent, _, cx| this.tool_up(cx)),
            )
            .when(
                self.retouch.tool != crate::retouch::Tool::Off && self.output.is_some(),
                |d| d.cursor_crosshair(),
            )
            .when(self.viewer.panning.is_some() || self.viewer.space, |d| {
                d.cursor_grab()
            });
        if self.gallery.open && matches!(self.doc, Doc::Ready { .. }) {
            return base.child(self.gallery_view(t, cx));
        }
        if self.variations.open && matches!(self.doc, Doc::Ready { .. }) {
            return base.child(self.variations_view(t, cx));
        }
        if let Doc::Ready { image, .. } = &self.doc {
            let bg = [t.base.r, t.base.g, t.base.b].map(|v| (v * 255.0).round() as u8);
            let shown = self.retouched().unwrap_or_else(|| image.clone());
            let before = self
                .output
                .as_ref()
                .and(self.viewer.compare)
                .map(|_| image.clone());
            return base.child(self.viewer.element(shown, before, bg));
        }
        base.flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .p_4()
            .text_size(theme::size::large())
            .text_color(t.muted)
            .child(svg().path(icon("open")).size_8().text_color(t.border))
            .child(div().max_w_full().text_center().child(text))
            .when(!recents.is_empty(), |d| {
                d.child(div().mt_4().text_size(theme::size::small()).child("recent"))
                    .child(div().flex().flex_col().items_center().children(recents))
            })
    }

    fn status_bar(&self, t: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let mut parts: Vec<String> = match &self.doc {
            Doc::Ready { image, .. } => status_parts(image, self.output.as_deref(), self),
            Doc::Loading(_) => vec!["loading".into()],
            Doc::Failed(_) => vec!["error".into()],
            Doc::Empty => Vec::new(),
        };
        if let Some(n) = &self.notice {
            parts.push(n.to_string());
        }
        let text = parts.join("   ");
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
            .gap_3()
            .child(text)
            .when(self.approximate && self.output.is_some(), |d| {
                d.child(
                    div()
                        .id("force-exact")
                        .px_1()
                        .rounded(px(3.0))
                        .text_color(t.accent)
                        .cursor_pointer()
                        .hover(|d| d.bg(t.elevated))
                        .on_click(cx.listener(|this, _, _, cx| this.convert(false, cx)))
                        .child("preview, click for exact"),
                )
            })
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

/// Loads a still image, or the frames of a GIF (the first frame is returned as the
/// image too).
fn load_any(path: &std::path::Path) -> xd_art_core::Result<(Image, Vec<crate::frames::Frame>)> {
    if kind(path) == Kind::Image
        && path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("gif"))
    {
        let a = xd_art_core::anim::load_gif(path)?;
        let frames: Vec<crate::frames::Frame> = a
            .frames
            .into_iter()
            .map(|f| crate::frames::Frame {
                image: Arc::new(f.image),
                delay: f.delay,
                skip: false,
            })
            .collect();
        let first = frames
            .first()
            .map(|f| (*f.image).clone())
            .ok_or(xd_art_core::Error::Cancelled)?;
        let frames = if frames.len() > 1 { frames } else { Vec::new() };
        return Ok((first, frames));
    }
    Ok((xd_art_core::io::load(path)?, Vec::new()))
}

/// What the status bar says about an open image: sizes, colors, dither mode, time of
/// each stage that ran, and the memory held by the image, the result and the cache.
fn status_parts(image: &Image, out: Option<&Image>, root: &Root) -> Vec<String> {
    let mut v = vec![format!("{} x {}", image.width(), image.height())];
    let Some(out) = out else { return v };
    v[0] = format!("{} -> {} x {}", v[0], out.width(), out.height());
    let used = root.strip.counts.iter().filter(|&&n| n > 0).count();
    v.push(format!("{used} colors"));
    v.push(root.settings.dither.mode.name().to_string());
    if let Some((took, report)) = &root.last_run {
        let stages: Vec<String> = Stage::ALL
            .iter()
            .filter(|s| report.ran.contains(**s))
            .map(|s| {
                format!(
                    "{} {:.1}",
                    s.name(),
                    report.times[*s as usize].as_secs_f64() * 1000.0
                )
            })
            .collect();
        v.push(format!("{:.0} ms", took.as_secs_f64() * 1000.0));
        if !stages.is_empty() {
            v.push(stages.join(" "));
        }
    }
    let cache = root
        .pipeline
        .try_lock()
        .map(|p| p.older_bytes())
        .unwrap_or(0);
    let held = image.heap_size() + out.heap_size() + cache;
    v.push(format!("{:.1} MB", held as f64 / (1 << 20) as f64));
    v
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
        let image_only = self.fullscreen;
        let mut middle = div().flex_1().flex().min_h_0();
        if self.layout.left_open && !image_only {
            middle = middle.child(self.panel(Side::Left, t, cx));
        }
        middle = middle.child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(self.preview(t, cx))
                .when(!self.frames.list.is_empty() && !image_only, |d| {
                    d.child(self.frame_strip(t, cx))
                })
                .when(self.output.is_some() && !image_only, |d| {
                    d.child(self.palette_strip(t, cx))
                }),
        );
        if self.layout.right_open && !image_only {
            middle = middle.child(self.panel(Side::Right, t, cx));
        }
        div()
            .id("root")
            .relative()
            .track_focus(&self.focus)
            .key_context(actions::CONTEXT)
            .on_action(cx.listener(|this, _: &Open, window, cx| this.prompt_open(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleLeft, _, cx| this.toggle(Side::Left, cx)))
            .on_action(cx.listener(|this, _: &ToggleRight, _, cx| this.toggle(Side::Right, cx)))
            .on_action(cx.listener(|this, _: &Fit, _, cx| {
                this.viewer.fit();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ActualSize, _, cx| {
                if let Some(size) = this.image_size() {
                    this.viewer.actual_size(size);
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| {
                this.viewer.zoom_step(true);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| {
                this.viewer.zoom_step(false);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Gallery, _, cx| this.toggle_gallery(cx)))
            .on_action(cx.listener(|this, _: &Fullscreen, window, cx| {
                this.fullscreen = !this.fullscreen;
                if window.is_fullscreen() != this.fullscreen {
                    window.toggle_fullscreen();
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ReduceMotion, _, cx| {
                this.config.reduced_motion = !this.config.reduced_motion;
                this.notice = Some(
                    if this.config.reduced_motion {
                        "transitions off"
                    } else {
                        "transitions on"
                    }
                    .into(),
                );
                this.persist(cx);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, window, cx| {
                if this.retouch.tool != crate::retouch::Tool::Off
                    && e.keystroke.key == "escape"
                    && !this.fullscreen
                {
                    this.retouch.tool = crate::retouch::Tool::Off;
                    cx.notify();
                    return;
                }
                if this.fullscreen && e.keystroke.key == "escape" {
                    this.fullscreen = false;
                    if window.is_fullscreen() {
                        window.toggle_fullscreen();
                    }
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &NextTheme, _, cx| {
                let v = Theme::get(cx).variant;
                let next =
                    theme::Variant::from_index((v.index() + 1) % theme::Variant::ALL.len() as u8);
                Theme::set(next, cx);
                this.apply_theme(cx);
                this.notice = Some(format!("theme: {}", next.name()).into());
                this.persist(cx);
            }))
            .on_action(cx.listener(|this, _: &AccentFromImage, _, cx| {
                this.config.accent_from_image = !this.config.accent_from_image;
                this.apply_theme(cx);
                this.persist(cx);
            }))
            .on_action(cx.listener(|this, _: &CommandPalette, _, cx| {
                this.commands.open = !this.commands.open;
                this.commands.sheet = false;
                this.commands.query.clear();
                this.commands.selected = 0;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ShortcutSheet, _, cx| {
                this.commands.sheet = !this.commands.sheet;
                this.commands.open = false;
                cx.notify();
            }))
            .capture_key_down(cx.listener(Self::command_key))
            .on_action(cx.listener(|this, _: &Variations, _, cx| {
                this.variations.open = !this.variations.open;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &KeepVariation, _, cx| {
                let n = this.variations.items.len() + 1;
                this.keep_variation(format!("version {n}"), this.settings.clone(), cx);
            }))
            .on_action(cx.listener(|this, _: &Undo, _, cx| {
                if this.retouch.tool.paints() && this.retouch.undo() {
                    cx.notify();
                    return;
                }
                if let Some(s) = this.history.undo() {
                    this.restore(s, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Redo, _, cx| {
                if this.retouch.tool.paints() && this.retouch.redo() {
                    cx.notify();
                    return;
                }
                if let Some(s) = this.history.redo() {
                    this.restore(s, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Compare, _, cx| {
                this.viewer.cycle_compare();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleGrid, _, cx| {
                this.viewer.grid = !this.viewer.grid;
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, _, cx| {
                if e.keystroke.key == "space" && !this.viewer.space {
                    this.viewer.space = true;
                    cx.notify();
                }
            }))
            .on_key_up(cx.listener(|this, e: &gpui::KeyUpEvent, _, cx| {
                if e.keystroke.key == "space" {
                    this.viewer.space = false;
                    cx.notify();
                }
            }))
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
                    this.viewer.panning = None;
                    this.viewer.dragging_split = false;
                    if this.resizing.take().is_some() {
                        this.persist(cx);
                        cx.notify();
                    }
                }),
            )
            .when(!image_only, |d| d.child(self.top_bar(t, cx)))
            .child(middle)
            .when(!image_only, |d| {
                d.child(self.status_bar(t, cx)).child(Self::footer(t, cx))
            })
            .when(image_only && self.edge_hover, |d| {
                d.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .child(self.top_bar(t, cx)),
                )
            })
            .when(self.commands.open, |d| d.child(self.command_palette(t, cx)))
            .when(self.commands.sheet, |d| d.child(self.shortcut_sheet(t)))
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
