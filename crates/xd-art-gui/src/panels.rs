//! The settings panel: one section per pipeline stage, in the order they run.
//!
//! A control only shows when it has an effect with the current settings: the edge
//! strength only for edge sampling, the dither amount only for ordered dithering, the
//! palette size only when the palette is built from the image, and so on.

use gpui::{AnyElement, Context, SharedString, div, prelude::*, px, svg};
use xd_art_core::settings::{
    DitherMode, Metric, PaletteSource, Quantizer, SampleMode, Settings, Size,
};

use crate::app::Root;
use crate::assets::icon;
use crate::theme::{self, Palette};

/// The sections, in pipeline order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    /// The source image.
    Input,
    /// Blur before sampling.
    Prepare,
    /// Size and sampling mode.
    Sampling,
    /// Color adjustments and the palette.
    Color,
    /// Dithering.
    Dithering,
    /// Outline, inline, cleanup.
    Post,
    /// Scale of the result.
    Output,
}

impl Section {
    /// Every section, in order.
    pub const ALL: [Self; 7] = [
        Self::Input,
        Self::Prepare,
        Self::Sampling,
        Self::Color,
        Self::Dithering,
        Self::Post,
        Self::Output,
    ];

    /// The heading.
    pub fn title(self) -> &'static str {
        match self {
            Self::Input => "Input",
            Self::Prepare => "Prepare",
            Self::Sampling => "Sampling",
            Self::Color => "Color",
            Self::Dithering => "Dithering",
            Self::Post => "Post",
            Self::Output => "Output",
        }
    }
}

/// A choice between a few named options.
pub struct Choice {
    /// Identifies the open list.
    pub id: &'static str,
    /// Label on the left.
    pub label: &'static str,
    /// Section it belongs to.
    pub section: Section,
    /// The option names.
    pub options: fn() -> Vec<SharedString>,
    /// Index of the current option.
    pub current: fn(&Settings) -> usize,
    /// Applies an option.
    pub apply: fn(&mut Settings, usize),
    /// Whether it has an effect right now.
    pub visible: fn(&Settings) -> bool,
}

/// An on/off setting.
pub struct Toggle {
    /// Identifies the element.
    pub id: &'static str,
    /// Label.
    pub label: &'static str,
    /// Section it belongs to.
    pub section: Section,
    /// Current state.
    pub get: fn(&Settings) -> bool,
    /// Sets the state.
    pub set: fn(&mut Settings, bool),
    /// Whether it has an effect right now.
    pub visible: fn(&Settings) -> bool,
}

fn always(_: &Settings) -> bool {
    true
}

fn names<T: Copy>(all: &[T], f: fn(T) -> &'static str) -> Vec<SharedString> {
    all.iter().map(|m| SharedString::from(f(*m))).collect()
}

fn palette_names() -> Vec<SharedString> {
    let mut v: Vec<SharedString> = xd_art_core::palette::BUILTINS
        .iter()
        .map(|b| SharedString::from(b.name))
        .collect();
    v.push("from the image".into());
    v.push("loaded file".into());
    v
}

fn palette_index(s: &Settings) -> usize {
    let n = xd_art_core::palette::BUILTINS.len();
    match &s.colors.palette {
        PaletteSource::Builtin(name) => xd_art_core::palette::BUILTINS
            .iter()
            .position(|b| b.name == name)
            .unwrap_or(0),
        PaletteSource::Extract { .. } => n,
        PaletteSource::Colors(_) => n + 1,
    }
}

fn set_palette(s: &mut Settings, i: usize) {
    let b = xd_art_core::palette::BUILTINS;
    if let Some(p) = b.get(i) {
        s.colors.palette = PaletteSource::Builtin(p.name.into());
    } else if i == b.len() && !matches!(s.colors.palette, PaletteSource::Extract { .. }) {
        s.colors.palette = PaletteSource::Extract {
            count: 16,
            method: Quantizer::KMeans,
            locked: Vec::new(),
        };
    }
    // "loaded file" is only a label for colors that came from a file or the palette
    // editor; picking it does not change anything.
}

const QUANTIZERS: [Quantizer; 3] = [Quantizer::KMeans, Quantizer::MedianCut, Quantizer::Balanced];

fn quantizer_name(q: Quantizer) -> &'static str {
    match q {
        Quantizer::KMeans => "k-means",
        Quantizer::MedianCut => "median cut",
        Quantizer::Balanced => "balanced",
    }
}

fn extracting(s: &Settings) -> bool {
    matches!(s.colors.palette, PaletteSource::Extract { .. })
}

fn preset_names() -> Vec<SharedString> {
    let mut v = vec![SharedString::from("custom")];
    v.extend(
        xd_art_core::preset::PRESETS
            .iter()
            .map(|p| SharedString::from(format!("{}: {}", p.name, p.about))),
    );
    v
}

/// Every choice in the panel.
pub fn choices() -> Vec<Choice> {
    vec![
        Choice {
            id: "preset",
            label: "start from",
            section: Section::Input,
            options: preset_names,
            current: |s| {
                xd_art_core::preset::PRESETS
                    .iter()
                    .position(|p| p.settings() == *s)
                    .map_or(0, |i| i + 1)
            },
            apply: |s, i| {
                if let Some(p) = i
                    .checked_sub(1)
                    .and_then(|i| xd_art_core::preset::PRESETS.get(i))
                {
                    *s = p.settings();
                }
            },
            visible: always,
        },
        Choice {
            id: "sample-mode",
            label: "mode",
            section: Section::Sampling,
            options: || names(&SampleMode::ALL, SampleMode::name),
            current: |s| {
                SampleMode::ALL
                    .iter()
                    .position(|m| *m == s.sample.mode)
                    .unwrap_or(0)
            },
            apply: |s, i| s.sample.mode = SampleMode::ALL[i],
            visible: always,
        },
        Choice {
            id: "size-kind",
            label: "size",
            section: Section::Sampling,
            options: || vec!["fixed".into(), "fixed width".into(), "divide".into()],
            current: |s| match s.sample.size {
                Size::Absolute {
                    keep_aspect: false, ..
                } => 0,
                Size::Absolute {
                    keep_aspect: true, ..
                } => 1,
                Size::Relative { .. } => 2,
            },
            apply: |s, i| {
                let w = match s.sample.size {
                    Size::Absolute { width, .. } => width,
                    Size::Relative { .. } => 64,
                };
                s.sample.size = match i {
                    0 => Size::Absolute {
                        width: w,
                        height: w,
                        keep_aspect: false,
                    },
                    1 => Size::Absolute {
                        width: w,
                        height: w,
                        keep_aspect: true,
                    },
                    _ => Size::Relative { x: 4, y: 4 },
                };
            },
            visible: always,
        },
        Choice {
            id: "palette",
            label: "palette",
            section: Section::Color,
            options: palette_names,
            current: palette_index,
            apply: set_palette,
            visible: always,
        },
        Choice {
            id: "quantizer",
            label: "method",
            section: Section::Color,
            options: || names(&QUANTIZERS, quantizer_name),
            current: |s| match &s.colors.palette {
                PaletteSource::Extract { method, .. } => {
                    QUANTIZERS.iter().position(|q| q == method).unwrap_or(0)
                }
                _ => 0,
            },
            apply: |s, i| {
                if let PaletteSource::Extract { method, .. } = &mut s.colors.palette {
                    *method = QUANTIZERS[i];
                }
            },
            visible: extracting,
        },
        Choice {
            id: "metric",
            label: "distance",
            section: Section::Color,
            options: || names(&Metric::ALL, Metric::name),
            current: |s| {
                Metric::ALL
                    .iter()
                    .position(|m| *m == s.colors.metric)
                    .unwrap_or(0)
            },
            apply: |s, i| s.colors.metric = Metric::ALL[i],
            visible: always,
        },
        Choice {
            id: "dither",
            label: "mode",
            section: Section::Dithering,
            options: || names(&DitherMode::ALL, DitherMode::name),
            current: |s| {
                DitherMode::ALL
                    .iter()
                    .position(|m| *m == s.dither.mode)
                    .unwrap_or(0)
            },
            apply: |s, i| s.dither.mode = DitherMode::ALL[i],
            visible: always,
        },
        Choice {
            id: "cleanup",
            label: "stray pixels",
            section: Section::Post,
            options: || {
                vec![
                    "keep".into(),
                    "remove single dots".into(),
                    "remove more".into(),
                ]
            },
            current: |s| usize::from(s.post.cleanup.min(2)),
            apply: |s, i| s.post.cleanup = i as u8,
            visible: always,
        },
    ]
}

/// Every toggle in the panel.
pub fn toggles() -> Vec<Toggle> {
    vec![
        Toggle {
            id: "blur-compat",
            label: "original blur",
            section: Section::Prepare,
            get: |s| s.blur.box_approx,
            set: |s, v| s.blur.box_approx = v,
            visible: |s| s.blur.radius > 0.0,
        },
        Toggle {
            id: "outline",
            label: "outline (color 0)",
            section: Section::Post,
            get: |s| s.post.outline.is_some(),
            set: |s, v| s.post.outline = v.then_some(0),
            visible: always,
        },
        Toggle {
            id: "inline",
            label: "inline (color 1)",
            section: Section::Post,
            get: |s| s.post.inline.is_some(),
            set: |s, v| s.post.inline = v.then_some(1),
            visible: always,
        },
    ]
}

impl Root {
    /// The content of the settings panel.
    pub(crate) fn settings_panel(&self, t: Palette, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        for (i, section) in Section::ALL.into_iter().enumerate() {
            let open = !self.collapsed[i];
            out.push(
                div()
                    .id(("section", i))
                    .flex()
                    .items_center()
                    .gap_1()
                    .mt_1()
                    .cursor_pointer()
                    .text_size(theme::size::title())
                    .text_color(t.text)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.collapsed[i] = !this.collapsed[i];
                        cx.notify();
                    }))
                    .child(
                        svg()
                            .path(icon(if open {
                                "chevron-down"
                            } else {
                                "chevron-right"
                            }))
                            .size(px(12.0))
                            .text_color(t.muted),
                    )
                    .child(section.title())
                    .into_any_element(),
            );
            if !open {
                continue;
            }
            let mut body: Vec<AnyElement> = Vec::new();
            if section == Section::Input {
                body.extend(self.input_rows(t));
            }
            for c in self
                .choices
                .iter()
                .filter(|c| c.section == section && (c.visible)(&self.settings))
            {
                body.push(self.choice(c, t, cx));
            }
            for g in self
                .toggles
                .iter()
                .filter(|g| g.section == section && (g.visible)(&self.settings))
            {
                body.push(self.toggle_row(g, t, cx));
            }
            for c in self
                .controls
                .iter()
                .filter(|c| c.section == section && (c.visible)(&self.settings))
            {
                body.push(c.slider.clone().into_any_element());
            }
            out.push(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .pl_4()
                    .children(body)
                    .into_any_element(),
            );
        }
        out
    }

    fn choice(&self, c: &Choice, t: Palette, cx: &mut Context<Self>) -> AnyElement {
        let options = (c.options)();
        let current = (c.current)(&self.settings).min(options.len().saturating_sub(1));
        let open = self.open_choice == Some(c.id);
        let id = c.id;
        let apply = c.apply;
        let header = div()
            .id(SharedString::from(format!("{id}-head")))
            .flex()
            .justify_between()
            .items_center()
            .px_2()
            .py(px(3.0))
            .rounded(px(4.0))
            .bg(t.elevated)
            .border_1()
            .border_color(if open { t.accent } else { t.border })
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open_choice = if this.open_choice == Some(id) {
                    None
                } else {
                    Some(id)
                };
                cx.notify();
            }))
            .child(div().text_color(t.muted).child(c.label))
            .child(
                div()
                    .text_color(t.text)
                    .child(options.get(current).cloned().unwrap_or_default()),
            );
        let mut col = div().flex().flex_col().gap(px(2.0)).child(header);
        if open {
            col = col.child(
                div()
                    .flex()
                    .flex_col()
                    .p_1()
                    .rounded(px(4.0))
                    .bg(t.elevated)
                    .border_1()
                    .border_color(t.border)
                    .children(options.into_iter().enumerate().map(|(i, name)| {
                        div()
                            .id(SharedString::from(format!("{id}-{i}")))
                            .px_2()
                            .py(px(2.0))
                            .rounded(px(3.0))
                            .cursor_pointer()
                            .text_color(if i == current { t.accent } else { t.text })
                            .hover(|d| d.bg(t.border))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                apply(&mut this.settings, i);
                                this.open_choice = None;
                                this.hover_dither(None, cx);
                                this.settings_changed(false, cx);
                            }))
                            .when(id == "dither", |d| {
                                d.on_hover(cx.listener(move |this, on: &bool, _, cx| {
                                    let now = this.gallery.hover.as_ref().map(|(h, _)| *h);
                                    if *on {
                                        this.hover_dither(Some(i), cx);
                                    } else if now == Some(i) {
                                        this.hover_dither(None, cx);
                                    }
                                }))
                            })
                            .child(name)
                    })),
            );
        }
        if open && id == "dither" {
            col = col.children(self.dither_hint_view(t));
        }
        if open && id == "sample-mode" {
            col = col.child(
                div()
                    .px_2()
                    .text_size(theme::size::small())
                    .text_color(t.muted)
                    .child(crate::gallery::sample_hint(self.settings.sample.mode)),
            );
        }
        col.into_any_element()
    }

    fn toggle_row(&self, g: &Toggle, t: Palette, cx: &mut Context<Self>) -> AnyElement {
        let on = (g.get)(&self.settings);
        let set = g.set;
        div()
            .id(g.id)
            .flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                set(&mut this.settings, !on);
                this.settings_changed(false, cx);
            }))
            .child(
                div()
                    .size(px(14.0))
                    .rounded(px(3.0))
                    .border_1()
                    .border_color(if on { t.accent } else { t.border })
                    .when(on, |d| d.bg(t.accent)),
            )
            .child(div().text_color(t.muted).child(g.label))
            .into_any_element()
    }
}
