//! The export panel (Ctrl+S): every way to write the result out.
//!
//! - PNG of the result, with the optional scanline and CRT effects.
//! - GIF: the animation when the source is an animated GIF (skipped frames left out),
//!   a still GIF otherwise.
//! - Sprite sheet: every frame packed into one PNG as a grid or a strip, with a JSON
//!   file of positions and durations and a `.gpl` palette next to it.
//! - Block texture: a PNG named the way resource packs expect (`stone_wall_16.png`).
//! - Handoff: indexed PNG, `.gpl`, `.hex` and a JSON file with the recipe, for
//!   continuing in a pixel art editor.
//!
//! The export scale is set here too. Retouches are part of every still export; GIF
//! animations and their sheets are made from the frames and leave them out.
//!
//! Also here: saving and loading recipes, and importing a palette from Lospec when the
//! build has the `online-palettes` feature.

use std::path::PathBuf;
use std::sync::Arc;

use gpui::{
    AnyElement, Context, KeyDownEvent, PathPromptOptions, SharedString, Window, div, prelude::*, px,
};
use xd_art_core::buffer::Image;
use xd_art_core::cancel::Cancel;
use xd_art_core::export::{Effects, SheetLayout};
use xd_art_core::pipeline::Pipeline;
use xd_art_core::settings::Settings;

use crate::app::Root;
use crate::retouch::{self, Layer};
use crate::theme::{self, Palette};
use xd_art_text::{fill, mark, tr};

/// Export panel state.
#[derive(Debug)]
pub struct Export {
    /// The panel is shown.
    pub open: bool,
    /// Effects for PNG export.
    pub effects: Effects,
    /// Typed Lospec name or link, while that field is active.
    pub lospec: Option<String>,
    /// Sprite sheet layout.
    pub sheet: SheetLayout,
}

impl Default for Export {
    fn default() -> Self {
        Self {
            open: false,
            effects: Effects::default(),
            lospec: None,
            sheet: SheetLayout {
                columns: 8,
                spacing: 1,
            },
        }
    }
}

/// Sheet layouts offered: label and columns (0 is one row).
const SHEET_COLUMNS: [(&str, u32); 3] = [(mark("grid"), 8), (mark("row"), 0), (mark("column"), 1)];

/// What to write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The result as PNG.
    Png,
    /// Animated or still GIF.
    Gif,
    /// Frames packed in one PNG plus JSON.
    Sheet,
    /// A PNG named for resource packs.
    Texture,
    /// Files for a pixel art editor.
    Handoff,
}

/// Everything a background export needs.
struct Job {
    kind: Kind,
    path: PathBuf,
    settings: Settings,
    effects: Effects,
    sheet: SheetLayout,
    still: Arc<Image>,
    frames: Vec<(Arc<Image>, u16)>,
    retouch: Option<Layer>,
}

/// An indexed result, its palette and its colors.
type Converted = (xd_art_core::buffer::Indexed, Vec<[u8; 3]>, Image);

impl Job {
    /// The still image converted, with the retouches over it.
    fn still(&self) -> xd_art_core::Result<Converted> {
        let (mut ix, pal, mut out) = convert(&self.still, &self.settings)?;
        if let Some(l) = &self.retouch {
            if let Some(img) = retouch::compose(&out, l, &pal) {
                out = img;
            }
            retouch::apply_indexed(&mut ix, l, &pal);
        }
        Ok((ix, pal, out))
    }
}

fn convert(img: &Image, s: &Settings) -> xd_art_core::Result<Converted> {
    let mut p = Pipeline::default();
    p.run(img, 1, s, &Cancel::new())?;
    let ix = p.indexed().cloned().ok_or(xd_art_core::Error::Cancelled)?;
    let pal = p
        .palette()
        .map(<[[u8; 3]]>::to_vec)
        .ok_or(xd_art_core::Error::Cancelled)?;
    let out = p.output().cloned().ok_or(xd_art_core::Error::Cancelled)?;
    Ok((ix, pal, out))
}

fn run(job: &Job) -> xd_art_core::Result<String> {
    let io_err = |path: &std::path::Path| {
        let path = path.to_path_buf();
        move |source| xd_art_core::Error::Io {
            path: path.clone(),
            source,
        }
    };
    match job.kind {
        Kind::Png | Kind::Texture => {
            let (_, _, out) = job.still()?;
            let out = job.effects.apply(&out, job.settings.output.scale)?;
            xd_art_core::io::save(&out, &job.path)?;
        }
        Kind::Gif if job.frames.is_empty() => {
            let (ix, pal, _) = job.still()?;
            let c = xd_art_core::anim::Converted {
                frames: vec![ix],
                palettes: vec![pal],
                delays: vec![0],
            };
            let bytes = xd_art_core::anim::encode_gif(&c, job.settings.output.scale)?;
            std::fs::write(&job.path, bytes).map_err(io_err(&job.path))?;
        }
        Kind::Gif => {
            let frames: Vec<xd_art_core::anim::Frame> = {
                job.frames
                    .iter()
                    .map(|(f, d)| xd_art_core::anim::Frame {
                        image: (**f).clone(),
                        delay: *d,
                    })
                    .collect()
            };
            let (w, h) = (frames[0].image.width(), frames[0].image.height());
            let a = xd_art_core::anim::Animation {
                width: w,
                height: h,
                frames,
            };
            let c = xd_art_core::anim::convert(
                &a,
                &job.settings,
                xd_art_core::anim::PaletteMode::Shared,
                &Cancel::new(),
            )?;
            let bytes = xd_art_core::anim::encode_gif(&c, job.settings.output.scale)?;
            std::fs::write(&job.path, bytes).map_err(io_err(&job.path))?;
        }
        Kind::Sheet => {
            let mut imgs = Vec::new();
            let mut durations = Vec::new();
            let mut colors: Vec<[u8; 3]> = Vec::new();
            if job.frames.is_empty() {
                let (_, pal, out) = job.still()?;
                imgs.push(out);
                durations.push(0);
                colors = pal;
            }
            for (f, d) in &job.frames {
                let (_, pal, out) = convert(f, &job.settings)?;
                imgs.push(out);
                durations.push(u32::from(*d) * 10);
                for c in pal {
                    if !colors.contains(&c) {
                        colors.push(c);
                    }
                }
            }
            let sheet = xd_art_core::export::sprite_sheet(&imgs, &durations, job.sheet)?;
            xd_art_core::io::save(&sheet.image, &job.path)?;
            let name = job
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("sheet.png");
            let json = job.path.with_extension("json");
            std::fs::write(&json, sheet.to_json(name)).map_err(io_err(&json))?;
            // The palette on its own, for engines and editors that load one.
            let gpl = job.path.with_extension("gpl");
            let stem = job
                .path
                .file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or("sheet");
            xd_art_core::palette::Palette::new(stem.to_string(), colors)?.save(&gpl)?;
            return Ok(fill(
                tr("{0} frames in {1}, with {2} and {3}"),
                &[
                    &imgs.len(),
                    &job.path.display(),
                    &json.display(),
                    &gpl.display(),
                ],
            ));
        }
        Kind::Handoff => {
            let (ix, pal, _) = job.still()?;
            let stem = job
                .path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("art")
                .to_string();
            let dir = job.path.parent().map(PathBuf::from).unwrap_or_default();
            let palette = xd_art_core::palette::Palette::new(stem.clone(), pal)?;
            let h = xd_art_core::export::handoff(&ix, &palette, &job.settings, &dir, &stem)?;
            return Ok(fill(
                tr("{0} files in {1}"),
                &[&h.files.len(), &dir.display()],
            ));
        }
    }
    Ok(fill(tr("saved {0}"), &[&job.path.display()]))
}

impl Root {
    /// Asks where to write and exports there in the background.
    pub(crate) fn export(&mut self, kind: Kind, cx: &mut Context<Self>) {
        let Some(still) = self.full_source() else {
            return;
        };
        let frames: Vec<(Arc<Image>, u16)> = self
            .frames
            .list
            .iter()
            .filter(|f| !f.skip)
            .map(|f| (f.image.clone(), f.delay))
            .collect();
        let stem = self.title().to_string();
        let stem = stem
            .rsplit_once('.')
            .map_or(stem.as_str(), |(s, _)| s)
            .to_string();
        let name = match kind {
            Kind::Png => format!("{stem}-px.png"),
            Kind::Gif => format!("{stem}-px.gif"),
            Kind::Sheet => format!("{stem}-sheet.png"),
            Kind::Texture => {
                let side = self
                    .output
                    .as_ref()
                    .map_or(16, |o| o.width().max(o.height()) / self.output_scale.max(1));
                xd_art_core::export::texture_name(&stem, side)
            }
            Kind::Handoff => format!("{stem}.png"),
        };
        let dir = self
            .config
            .recents
            .first()
            .and_then(|p| p.parent())
            .map(PathBuf::from)
            .unwrap_or_default();
        let pick = cx.prompt_for_new_path(&dir, Some(&name));
        let (settings, effects, sheet) = (
            self.settings.clone(),
            self.export.effects,
            self.export.sheet,
        );
        let retouch = self.retouch.layer.clone().filter(|l| !l.is_empty());
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = pick.await else {
                return;
            };
            let job = Job {
                kind,
                path,
                settings,
                effects,
                sheet,
                still,
                frames,
                retouch,
            };
            let done = cx
                .background_executor()
                .spawn(async move { run(&job) })
                .await;
            this.update(cx, |root, cx| {
                root.notice = Some(done.unwrap_or_else(|e| e.to_string()).into());
                root.export.open = false;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn save_recipe(&mut self, cx: &mut Context<Self>) {
        let dir = std::env::current_dir().unwrap_or_default();
        let pick = cx.prompt_for_new_path(&dir, Some("recipe.json"));
        let text = xd_art_core::recipe::to_json(&self.settings);
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = pick.await else {
                return;
            };
            let done = std::fs::write(&path, text).map(|()| path);
            this.update(cx, |root, cx| {
                root.notice = Some(match done {
                    Ok(p) => fill(tr("recipe saved to {0}"), &[&p.display()]).into(),
                    Err(e) => e.to_string().into(),
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn load_recipe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pick = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
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

    /// Keys typed into the Lospec field.
    pub(crate) fn lospec_key(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = &mut self.export.lospec else {
            return;
        };
        match e.keystroke.key.as_str() {
            "enter" => {
                let name = text.clone();
                self.export.lospec = None;
                self.import_lospec(name, cx);
            }
            "escape" => self.export.lospec = None,
            "backspace" => {
                text.pop();
            }
            _ => match e.keystroke.key_char.as_deref() {
                Some(c) => text.push_str(c),
                None => return,
            },
        }
        cx.stop_propagation();
        cx.notify();
    }

    #[cfg(feature = "online-palettes")]
    fn import_lospec(&mut self, name: String, cx: &mut Context<Self>) {
        let cache = crate::config::path().and_then(|p| p.parent().map(|d| d.join("lospec")));
        self.notice = Some(fill(tr("asking lospec.com for {0}"), &[&name]).into());
        cx.spawn(async move |this, cx| {
            let got = cx
                .background_executor()
                .spawn(async move { xd_art_core::palette::lospec::fetch(&name, cache.as_deref()) })
                .await;
            this.update(cx, |root, cx| match got {
                Ok(p) => {
                    root.notice = Some(
                        fill(tr("{0} ({1} colors) from lospec.com"), &[&p.name, &p.len()]).into(),
                    );
                    root.settings.colors.palette =
                        xd_art_core::settings::PaletteSource::Colors(p.colors().to_vec());
                    root.settings_changed(false, cx);
                }
                Err(e) => {
                    root.notice = Some(e.to_string().into());
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    #[cfg(not(feature = "online-palettes"))]
    fn import_lospec(&mut self, _name: String, cx: &mut Context<Self>) {
        self.notice = Some(tr("this build has no lospec import (feature online-palettes)").into());
        cx.notify();
    }

    /// The export panel.
    pub(crate) fn export_panel(&self, t: Palette, cx: &mut Context<Self>) -> AnyElement {
        let row = |id: &'static str, label: &'static str, hint: &'static str| {
            div()
                .id(id)
                .flex()
                .justify_between()
                .px_2()
                .py_1()
                .rounded(px(3.0))
                .cursor_pointer()
                .hover(|d| d.bg(t.elevated))
                .child(div().text_color(t.text).child(tr(label)))
                .child(
                    div()
                        .text_color(t.muted)
                        .text_size(theme::size::small())
                        .child(tr(hint)),
                )
        };
        let toggle = |id: &'static str, label: &'static str, on: bool| {
            div()
                .id(id)
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .cursor_pointer()
                .child(
                    div()
                        .size(px(12.0))
                        .rounded(px(2.0))
                        .border_1()
                        .border_color(if on { t.accent } else { t.border })
                        .when(on, |d| d.bg(t.accent)),
                )
                .child(div().text_color(t.muted).child(tr(label)))
        };
        let chip = |id: SharedString, label: SharedString, on: bool| {
            div()
                .id(id)
                .px_2()
                .rounded(px(3.0))
                .text_size(theme::size::small())
                .cursor_pointer()
                .border_1()
                .border_color(if on { t.accent } else { t.border })
                .text_color(if on { t.text } else { t.muted })
                .hover(|d| d.bg(t.elevated))
                .child(label)
        };
        let animated = !self.frames.list.is_empty();
        let e = self.export.effects;
        div()
            .absolute()
            .top(px(60.0))
            .left_1_2()
            .ml(px(-210.0))
            .w(px(420.0))
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .rounded(px(6.0))
            .bg(t.panel)
            .border_1()
            .border_color(t.border)
            .text_size(theme::size::body())
            .child(
                div()
                    .text_size(theme::size::title())
                    .px_2()
                    .child(tr("Export")),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .child(
                        div()
                            .text_size(theme::size::small())
                            .text_color(t.muted)
                            .mr_1()
                            .child(tr("scale")),
                    )
                    .children([1u32, 2, 4, 8].map(|k| {
                        chip(
                            SharedString::from(format!("scale-{k}")),
                            format!("{k}x").into(),
                            self.settings.output.scale == k,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if this.settings.output.scale != k {
                                this.settings.output.scale = k;
                                this.settings_changed(false, cx);
                            }
                        }))
                    })),
            )
            .child(
                row(
                    "export-png",
                    mark("PNG"),
                    mark("the result, scaled, with the effects below"),
                )
                .on_click(cx.listener(|this, _, _, cx| this.export(Kind::Png, cx))),
            )
            .child(
                row(
                    "export-gif",
                    mark("GIF"),
                    if animated {
                        mark("the animation, skipped frames left out")
                    } else {
                        mark("a still GIF")
                    },
                )
                .on_click(cx.listener(|this, _, _, cx| this.export(Kind::Gif, cx))),
            )
            .child(
                row(
                    "export-sheet",
                    mark("sprite sheet"),
                    mark("frames in one PNG, plus JSON with positions"),
                )
                .on_click(cx.listener(|this, _, _, cx| this.export(Kind::Sheet, cx))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .child(
                        div()
                            .text_size(theme::size::small())
                            .text_color(t.muted)
                            .mr_1()
                            .child(tr("sheet")),
                    )
                    .children(SHEET_COLUMNS.iter().map(|&(label, columns)| {
                        chip(
                            SharedString::from(format!("sheet-{label}")),
                            tr(label).into(),
                            self.export.sheet.columns == columns,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.export.sheet.columns = columns;
                            cx.notify();
                        }))
                    }))
                    .child(
                        chip(
                            "sheet-spacing".into(),
                            tr("1 px apart").into(),
                            self.export.sheet.spacing > 0,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.export.sheet.spacing = u32::from(this.export.sheet.spacing == 0);
                            cx.notify();
                        })),
                    ),
            )
            .child(
                row(
                    "export-texture",
                    mark("block texture"),
                    mark("PNG named like oak_log_16.png, for resource packs"),
                )
                .on_click(cx.listener(|this, _, _, cx| this.export(Kind::Texture, cx))),
            )
            .child(
                row(
                    "export-handoff",
                    mark("for an editor"),
                    mark("indexed PNG, .gpl, .hex and JSON"),
                )
                .on_click(cx.listener(|this, _, _, cx| this.export(Kind::Handoff, cx))),
            )
            .child(
                div()
                    .flex()
                    .gap_3()
                    .py_1()
                    .child(
                        toggle("fx-scan", mark("scanlines"), e.scanlines > 0.0).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.export.effects.scanlines =
                                    if this.export.effects.scanlines > 0.0 {
                                        0.0
                                    } else {
                                        0.35
                                    };
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        toggle("fx-crt", mark("crt curve"), e.crt > 0.0).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.export.effects.crt = if this.export.effects.crt > 0.0 {
                                    0.0
                                } else {
                                    0.15
                                };
                                cx.notify();
                            },
                        )),
                    ),
            )
            .child(div().h(px(1.0)).bg(t.border).my_1())
            .child(
                row(
                    "recipe-save",
                    mark("save recipe"),
                    mark("these settings as a small JSON file"),
                )
                .on_click(cx.listener(|this, _, _, cx| this.save_recipe(cx))),
            )
            .child(
                row(
                    "recipe-load",
                    mark("load recipe or palette"),
                    mark("also works by dropping the file"),
                )
                .on_click(cx.listener(|this, _, window, cx| this.load_recipe(window, cx))),
            )
            .child(
                row(
                    "lospec",
                    mark("palette from lospec.com"),
                    mark("type a name or link, enter"),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.export.lospec = Some(String::new());
                    cx.notify();
                })),
            )
            .when_some(self.export.lospec.clone(), |d, text| {
                d.child(
                    div()
                        .mx_2()
                        .px_2()
                        .py_1()
                        .rounded(px(3.0))
                        .bg(t.base)
                        .border_1()
                        .border_color(t.accent)
                        .child(SharedString::from(format!("{text}|"))),
                )
            })
            .child(
                div()
                    .px_2()
                    .text_size(theme::size::small())
                    .text_color(t.muted)
                    .child(tr("escape closes")),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(kind: Kind, path: PathBuf, frames: usize) -> Job {
        let mut img = Image::new(8, 8, [0, 0, 0]).unwrap();
        for i in 0..8 {
            img.set(i, i, [255, 255, 255, 255]);
        }
        let mut settings = Settings::default();
        settings.sample.size = xd_art_core::settings::Size::Absolute {
            width: 4,
            height: 4,
            keep_aspect: false,
        };
        let still = Arc::new(img);
        // One retouched pixel at (0, 3).
        let mut paint = Image::from_rgba(4, 4, &[0; 64]).unwrap();
        paint.set(0, 3, [255, 0, 77, 255]);
        let layer = Layer::from_image(&paint);
        Job {
            kind,
            path,
            settings,
            effects: Effects::default(),
            sheet: Export::default().sheet,
            frames: (0..frames).map(|_| (still.clone(), 5)).collect(),
            still,
            retouch: Some(layer),
        }
    }

    #[test]
    fn every_kind_writes_its_files() {
        let dir = std::env::temp_dir().join(format!("xd-art-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (kind, name, frames) in [
            (Kind::Png, "a.png", 0),
            (Kind::Gif, "a.gif", 0),
            (Kind::Gif, "b.gif", 3),
            (Kind::Sheet, "s.png", 3),
            (Kind::Texture, "t_4.png", 0),
            (Kind::Handoff, "h.png", 0),
        ] {
            let path = dir.join(name);
            run(&job(kind, path.clone(), frames)).unwrap();
            assert!(path.is_file(), "{name}");
        }
        assert!(dir.join("s.json").is_file());
        assert!(dir.join("s.gpl").is_file());
        // The retouched pixel is in the PNG; PICO-8 has the exact color.
        let png = xd_art_core::io::load(&dir.join("a.png")).unwrap();
        assert_eq!(png.get(0, 3), Some([255, 0, 77, 255]));
        std::fs::remove_dir_all(&dir).ok();
    }
}
