//! Retouching the converted image by hand, and painting the dithering mask.
//!
//! The pencil and the eraser work on a layer over the output: a painted pixel keeps its
//! color when settings change, the eraser gives the pixel back to the conversion. Only
//! palette colors can be painted, and a painted color that left the palette is drawn
//! with the nearest one still in it. The layer is in art pixels, so the export scale
//! does not matter. It belongs to one art size; when the size changes it is kept but
//! not drawn until the size matches again.
//!
//! The mask brushes paint the grayscale mask the engine reads (F6): "no dither" paints
//! black, "dither" paints white. Mask strokes are settings changes, so they go through
//! the normal history. Pencil strokes have their own undo list, used while the pencil
//! or the eraser is the active tool.
//!
//! "Save with image" writes the layer and the mask as PNG files next to the source
//! (`photo.png.retouch.png`, `photo.png.mask.png`), and opening the image loads them
//! back. That pair of files is what xd-art calls the project.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{AnyElement, Context, Pixels, Point, SharedString, div, prelude::*, px};
use xd_art_core::buffer::Image;
use xd_art_core::settings::Mask;

use crate::app::{Doc, Root};
use crate::theme::{self, Palette};

/// What a left click on the preview does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    /// Nothing: left drag does the usual things.
    #[default]
    Off,
    /// Paints the brush color.
    Pencil,
    /// Gives pixels back to the conversion.
    Eraser,
    /// Takes the brush color from the image.
    Picker,
    /// Paints the mask black: no dithering there.
    NoDither,
    /// Paints the mask white: full dithering.
    Dither,
}

impl Tool {
    const ALL: [Tool; 6] = [
        Tool::Off,
        Tool::Pencil,
        Tool::Eraser,
        Tool::Picker,
        Tool::NoDither,
        Tool::Dither,
    ];

    fn name(self) -> &'static str {
        match self {
            Tool::Off => "off",
            Tool::Pencil => "pencil",
            Tool::Eraser => "eraser",
            Tool::Picker => "picker",
            Tool::NoDither => "no dither",
            Tool::Dither => "dither",
        }
    }

    /// Whether the tool edits the layer (and so uses the layer's undo list).
    pub fn paints(self) -> bool {
        matches!(self, Tool::Pencil | Tool::Eraser)
    }

    /// Whether the tool paints the mask.
    pub fn masks(self) -> bool {
        matches!(self, Tool::NoDither | Tool::Dither)
    }
}

/// Painted pixels over an output of one size. `0` is untouched, anything else is
/// `0x0100_0000 | rgb`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layer {
    width: u32,
    height: u32,
    px: Vec<u32>,
}

const SET: u32 = 0x0100_0000;

impl Layer {
    /// An untouched layer.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            px: vec![0; width as usize * height as usize],
        }
    }

    /// Size of the output it was painted on.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Whether nothing is painted.
    pub fn is_empty(&self) -> bool {
        self.px.iter().all(|&p| p == 0)
    }

    /// Sets pixel `(x, y)` to `color` (`None` erases). Returns the index and the old
    /// value when something changed.
    fn put(&mut self, x: i64, y: i64, color: Option<[u8; 3]>) -> Option<(u32, u32)> {
        if x < 0 || y < 0 || x >= i64::from(self.width) || y >= i64::from(self.height) {
            return None;
        }
        let i = y as usize * self.width as usize + x as usize;
        let v = color.map_or(0, |c| {
            SET | u32::from(c[0]) << 16 | u32::from(c[1]) << 8 | u32::from(c[2])
        });
        let old = std::mem::replace(&mut self.px[i], v);
        (old != v).then_some((i as u32, old))
    }

    /// The layer as an RGBA image, transparent where untouched.
    pub fn to_image(&self) -> Option<Image> {
        let rgba: Vec<u8> = self
            .px
            .iter()
            .flat_map(|&p| {
                if p == 0 {
                    [0, 0, 0, 0]
                } else {
                    [(p >> 16) as u8, (p >> 8) as u8, p as u8, 255]
                }
            })
            .collect();
        Image::from_rgba(self.width, self.height, &rgba).ok()
    }

    /// Reads a layer written by [`Layer::to_image`]: opaque pixels are painted.
    pub fn from_image(img: &Image) -> Self {
        let mut l = Layer::new(img.width(), img.height());
        for y in 0..img.height() {
            for x in 0..img.width() {
                if let Some(p) = img.get(x, y).filter(|p| p[3] >= 128) {
                    l.put(i64::from(x), i64::from(y), Some([p[0], p[1], p[2]]));
                }
            }
        }
        l
    }
}

/// The pixels on the line from `a` to `b`, both ends included (Bresenham).
pub fn line(a: (i64, i64), b: (i64, i64)) -> Vec<(i64, i64)> {
    let (dx, dy) = ((b.0 - a.0).abs(), -(b.1 - a.1).abs());
    let (sx, sy) = ((b.0 - a.0).signum(), (b.1 - a.1).signum());
    let (mut x, mut y, mut err) = (a.0, a.1, dx + dy);
    let mut v = Vec::with_capacity((dx - dy) as usize + 1);
    loop {
        v.push((x, y));
        if (x, y) == b {
            return v;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

/// The pixels a square brush of side `size` covers when centered on `p`.
fn brush(p: (i64, i64), size: u32) -> impl Iterator<Item = (i64, i64)> {
    let s = i64::from(size.max(1));
    let start = (p.0 - (s - 1) / 2, p.1 - (s - 1) / 2);
    (0..s).flat_map(move |dy| (0..s).map(move |dx| (start.0 + dx, start.1 + dy)))
}

/// The nearest color of `palette` to `c`, or `c` itself when the palette is empty.
fn nearest(c: [u8; 3], palette: &[[u8; 3]]) -> [u8; 3] {
    palette
        .iter()
        .copied()
        .min_by_key(|p| {
            p.iter()
                .zip(c)
                .map(|(&a, b)| (i32::from(a) - i32::from(b)).pow(2))
                .sum::<i32>()
        })
        .unwrap_or(c)
}

/// How many output pixels one layer pixel covers on each side, when `out` is the
/// layer's size times a whole number.
pub fn scale_of(layer: (u32, u32), out: (u32, u32)) -> Option<u32> {
    let k = out.0.checked_div(layer.0)?;
    (k > 0 && layer.0 * k == out.0 && layer.1 * k == out.1).then_some(k)
}

/// `out` with `layer` drawn over it, every painted color moved into `palette`. `None`
/// when the layer does not fit `out` or paints nothing.
pub fn compose(out: &Image, layer: &Layer, palette: &[[u8; 3]]) -> Option<Image> {
    let k = scale_of(layer.size(), (out.width(), out.height()))?;
    if layer.is_empty() {
        return None;
    }
    let mut img = out.clone();
    for (i, &p) in layer.px.iter().enumerate() {
        if p != 0 {
            let c = nearest([(p >> 16) as u8, (p >> 8) as u8, p as u8], palette);
            let (x, y) = (i as u32 % layer.width * k, i as u32 / layer.width * k);
            for dy in 0..k {
                for dx in 0..k {
                    img.set(x + dx, y + dy, [c[0], c[1], c[2], 255]);
                }
            }
        }
    }
    Some(img)
}

/// `mask` (or a white one of `size` when there is none) with the brush painted to
/// `value` at every point, given in art pixels of an image of `size`.
pub fn paint_mask(
    mask: Option<&Mask>,
    size: (u32, u32),
    points: &[(i64, i64)],
    brush_size: u32,
    value: u8,
) -> Option<Mask> {
    let (mw, mh) = mask.map_or(size, Mask::size);
    let mut data = mask.map_or_else(
        || vec![255; mw as usize * mh as usize],
        |m| m.data().to_vec(),
    );
    let sx = f64::from(mw) / f64::from(size.0.max(1));
    let sy = f64::from(mh) / f64::from(size.1.max(1));
    for &p in points {
        for (x, y) in brush(p, brush_size) {
            if x < 0 || y < 0 || x >= i64::from(size.0) || y >= i64::from(size.1) {
                continue;
            }
            // Every mask pixel inside this output pixel.
            let x0 = (x as f64 * sx) as u32;
            let x1 = (((x + 1) as f64 * sx).ceil() as u32).clamp(x0 + 1, mw);
            let y0 = (y as f64 * sy) as u32;
            let y1 = (((y + 1) as f64 * sy).ceil() as u32).clamp(y0 + 1, mh);
            for my in y0..y1 {
                for mx in x0..x1 {
                    data[my as usize * mw as usize + mx as usize] = value;
                }
            }
        }
    }
    Mask::new(mw, mh, data).ok()
}

/// Where the project file of kind `kind` (`retouch` or `mask`) for `source` lives.
pub fn sidecar(source: &Path, kind: &str) -> PathBuf {
    let mut name = source.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{kind}.png"));
    source.with_file_name(name)
}

/// Layer pixels a stroke changed: index and the value before.
type Changes = Vec<(u32, u32)>;

type CacheKey = (usize, u64, usize);

/// Key, the output it was made from, and the composed image.
type Cached = (CacheKey, Arc<Image>, Arc<Image>);

/// Retouch state.
pub struct Retouch {
    /// The active tool.
    pub tool: Tool,
    /// Pencil color.
    pub color: Option<[u8; 3]>,
    /// Brush side in output pixels.
    pub size: u32,
    /// Painted pixels.
    pub layer: Option<Layer>,
    undo: Vec<Changes>,
    redo: Vec<Changes>,
    /// The stroke being drawn: its changes, and the last point for joining moves.
    stroke: Option<(Changes, (i64, i64))>,
    revision: u64,
    /// The last composed image, with the output, revision and palette it was made from.
    cache: RefCell<Option<Cached>>,
}

impl Default for Retouch {
    fn default() -> Self {
        Self {
            tool: Tool::Off,
            color: None,
            size: 1,
            layer: None,
            undo: Vec::new(),
            redo: Vec::new(),
            stroke: None,
            revision: 0,
            cache: RefCell::new(None),
        }
    }
}

impl Retouch {
    /// A fresh state for a newly opened image, keeping the tool and the brush.
    pub fn reset(&mut self, layer: Option<Layer>) {
        *self = Self {
            tool: self.tool,
            color: self.color,
            size: self.size,
            layer,
            ..Self::default()
        };
    }

    /// Whether a stroke is being drawn.
    pub fn drawing(&self) -> bool {
        self.stroke.is_some()
    }

    /// Starts a pencil or eraser stroke at `p` on art of `size`.
    pub fn begin(&mut self, p: (i64, i64), size: (u32, u32)) {
        if self.layer.as_ref().is_none_or(|l| l.size() != size) {
            let keep = self.layer.as_ref().is_some_and(|l| !l.is_empty());
            if keep {
                // Painting on another size would lose the old layer; the user has to
                // go back to that size or clear it first.
                return;
            }
            self.layer = Some(Layer::new(size.0, size.1));
            self.undo.clear();
        }
        self.stroke = Some((Vec::new(), p));
        self.extend(p);
    }

    /// Continues the stroke to `p`, joining it to the last point.
    pub fn extend(&mut self, p: (i64, i64)) {
        let (Some((changes, last)), Some(layer)) = (self.stroke.as_mut(), self.layer.as_mut())
        else {
            return;
        };
        let color = if self.tool == Tool::Eraser {
            None
        } else {
            self.color
        };
        if self.tool == Tool::Pencil && color.is_none() {
            return;
        }
        for q in line(*last, p) {
            for (x, y) in brush(q, self.size) {
                if let Some(c) = layer.put(x, y, color) {
                    changes.push(c);
                }
            }
        }
        *last = p;
        self.revision += 1;
    }

    /// Ends the stroke, keeping it for undo when it changed anything.
    pub fn end(&mut self) {
        if let Some((changes, _)) = self.stroke.take()
            && !changes.is_empty()
        {
            self.undo.push(changes);
            self.redo.clear();
        }
    }

    fn swap(&mut self, back: bool) -> bool {
        let (from, to) = if back {
            (&mut self.undo, &mut self.redo)
        } else {
            (&mut self.redo, &mut self.undo)
        };
        let (Some(changes), Some(layer)) = (from.pop(), self.layer.as_mut()) else {
            return false;
        };
        let mut reverse = Vec::with_capacity(changes.len());
        for &(i, v) in changes.iter().rev() {
            reverse.push((i, std::mem::replace(&mut layer.px[i as usize], v)));
        }
        to.push(reverse);
        self.revision += 1;
        true
    }

    /// Takes back the last stroke. False when there is none.
    pub fn undo(&mut self) -> bool {
        self.swap(true)
    }

    /// Puts back the last stroke taken back. False when there is none.
    pub fn redo(&mut self) -> bool {
        self.swap(false)
    }

    /// Removes every painted pixel, as one undoable step.
    pub fn clear(&mut self) {
        if let Some(layer) = self.layer.as_mut() {
            let changes: Vec<(u32, u32)> = layer
                .px
                .iter_mut()
                .enumerate()
                .filter(|(_, p)| **p != 0)
                .map(|(i, p)| (i as u32, std::mem::take(p)))
                .collect();
            if !changes.is_empty() {
                self.undo.push(changes);
                self.redo.clear();
                self.revision += 1;
            }
        }
    }

    /// `out` with the layer over it, composed once per change and then reused.
    pub fn show(&self, out: Arc<Image>, palette: &[[u8; 3]]) -> Arc<Image> {
        let Some(layer) = &self.layer else {
            return out;
        };
        let key = (
            Arc::as_ptr(&out) as usize,
            self.revision,
            palette.as_ptr() as usize ^ palette.len(),
        );
        if let Some((k, _, img)) = &*self.cache.borrow()
            && *k == key
        {
            return img.clone();
        }
        let img = compose(&out, layer, palette).map_or_else(|| out.clone(), Arc::new);
        // Holding `out` keeps its address from being reused by another output.
        *self.cache.borrow_mut() = Some((key, out, img.clone()));
        img
    }
}

impl Root {
    /// The output as shown and exported: the conversion with the retouches over it.
    pub(crate) fn retouched(&self) -> Option<Arc<Image>> {
        self.output
            .clone()
            .map(|o| self.retouch.show(o, &self.strip.colors))
    }

    /// The art size of the output (before the export scale) and that scale.
    fn art(&self) -> Option<((u32, u32), u32)> {
        let out = self.output.as_ref()?;
        let k = self.output_scale.max(1);
        Some(((out.width() / k, out.height() / k), k))
    }

    /// The art pixel under window position `pos`.
    fn art_pixel(&self, pos: Point<Pixels>, k: u32) -> (i64, i64) {
        let p = self.viewer.pixel_at(pos);
        let k = i64::from(k);
        (p.0.div_euclid(k), p.1.div_euclid(k))
    }

    /// Handles a left press on the preview. True when a tool used it.
    pub(crate) fn tool_down(&mut self, pos: Point<Pixels>, cx: &mut Context<Self>) -> bool {
        let tool = self.retouch.tool;
        let (Some(out), Some((size, k))) = (self.output.clone(), self.art()) else {
            return false;
        };
        let p = self.art_pixel(pos, k);
        match tool {
            Tool::Off => return false,
            Tool::Picker => {
                let shown = self.retouched().unwrap_or(out);
                if let Some(c) = u32::try_from(p.0)
                    .ok()
                    .zip(u32::try_from(p.1).ok())
                    .and_then(|(x, y)| shown.get(x * k, y * k))
                {
                    self.retouch.color = Some([c[0], c[1], c[2]]);
                    self.retouch.tool = Tool::Pencil;
                }
            }
            Tool::Pencil | Tool::Eraser => {
                if tool == Tool::Pencil && self.retouch.color.is_none() {
                    self.notice = Some("pick a palette color first".into());
                }
                self.retouch.begin(p, size);
                if !self.retouch.drawing() {
                    self.notice = Some(
                        "the retouches belong to another output size: clear them to paint here"
                            .into(),
                    );
                }
            }
            Tool::NoDither | Tool::Dither => {
                self.retouch.stroke = Some((Vec::new(), p));
                self.mask_points(&[p], size, cx);
            }
        }
        cx.notify();
        true
    }

    /// Mouse move with the button held. True when a stroke used it.
    pub(crate) fn tool_move(&mut self, pos: Point<Pixels>, cx: &mut Context<Self>) -> bool {
        if !self.retouch.drawing() {
            return false;
        }
        let Some((size, k)) = self.art() else {
            return true;
        };
        let p = self.art_pixel(pos, k);
        if self.retouch.tool.masks() {
            let last = self.retouch.stroke.as_ref().map_or(p, |s| s.1);
            if last != p {
                self.mask_points(&line(last, p), size, cx);
                if let Some(s) = self.retouch.stroke.as_mut() {
                    s.1 = p;
                }
            }
        } else {
            self.retouch.extend(p);
            cx.notify();
        }
        true
    }

    /// Button released: ends the stroke.
    pub(crate) fn tool_up(&mut self, cx: &mut Context<Self>) {
        if !self.retouch.drawing() {
            return;
        }
        if self.retouch.tool.masks() {
            self.retouch.stroke = None;
            self.settings_changed(false, cx);
        } else {
            self.retouch.end();
        }
        cx.notify();
    }

    fn mask_points(&mut self, points: &[(i64, i64)], size: (u32, u32), cx: &mut Context<Self>) {
        let value = if self.retouch.tool == Tool::NoDither {
            0
        } else {
            255
        };
        if let Some(m) = paint_mask(
            self.settings.mask.as_ref(),
            size,
            points,
            self.retouch.size,
            value,
        ) {
            self.settings.mask = Some(m);
            self.settings_changed(true, cx);
        }
    }

    /// Loads the project files next to `path`, if there are any.
    pub(crate) fn load_project(path: &Path) -> (Option<Layer>, Option<Mask>) {
        let read = |kind| {
            let p = sidecar(path, kind);
            p.is_file()
                .then(|| xd_art_core::io::load(&p).ok())
                .flatten()
        };
        let layer = read("retouch").map(|i| Layer::from_image(&i));
        let mask = read("mask").and_then(|i| Mask::from_image(&i).ok());
        (layer, mask)
    }

    /// Writes the layer and the mask next to the source image, or removes them when
    /// there is nothing to keep.
    fn save_project(&mut self, cx: &mut Context<Self>) {
        let Doc::Ready { path, .. } = &self.doc else {
            return;
        };
        let path = path.clone();
        let layer = self
            .retouch
            .layer
            .as_ref()
            .filter(|l| !l.is_empty())
            .and_then(Layer::to_image);
        let mask = self.settings.mask.as_ref().and_then(|m| {
            let (w, h) = m.size();
            let rgba: Vec<u8> = m.data().iter().flat_map(|&v| [v, v, v, 255]).collect();
            Image::from_rgba(w, h, &rgba).ok()
        });
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut written = 0;
                    for (kind, img) in [("retouch", layer), ("mask", mask)] {
                        let p = sidecar(&path, kind);
                        match img {
                            Some(img) => {
                                let bytes =
                                    xd_art_core::io::encode(&img, xd_art_core::io::Format::Png)
                                        .map_err(|e| e.to_string())?;
                                std::fs::write(&p, bytes)
                                    .map_err(|e| format!("{}: {e}", p.display()))?;
                                written += 1;
                            }
                            // An old file would come back on the next open.
                            None if p.is_file() => {
                                std::fs::remove_file(&p)
                                    .map_err(|e| format!("{}: {e}", p.display()))?;
                            }
                            None => {}
                        }
                    }
                    Ok::<_, String>(written)
                })
                .await;
            this.update(cx, |root, cx| {
                root.notice = Some(match result {
                    Ok(0) => "nothing to save: no retouches and no mask".into(),
                    Ok(n) => format!("saved {n} file(s) next to the image").into(),
                    Err(e) => e.into(),
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The Retouch section of the right panel.
    pub(crate) fn retouch_panel(&self, t: Palette, cx: &mut Context<Self>) -> Vec<AnyElement> {
        if self.output.is_none() {
            return Vec::new();
        }
        let button = |id: SharedString, label: SharedString, on: bool| {
            div()
                .id(id)
                .px_2()
                .py_1()
                .rounded(px(4.0))
                .text_size(theme::size::small())
                .cursor_pointer()
                .border_1()
                .border_color(if on { t.accent } else { t.border })
                .text_color(if on { t.text } else { t.muted })
                .hover(|d| d.bg(t.elevated))
                .child(label)
        };
        let tools = div()
            .flex()
            .flex_wrap()
            .gap_1()
            .children(Tool::ALL.map(|tool| {
                button(
                    format!("tool-{}", tool.name()).into(),
                    tool.name().into(),
                    self.retouch.tool == tool,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.retouch.tool = tool;
                    cx.notify();
                }))
            }));
        let sizes = div()
            .flex()
            .gap_1()
            .items_center()
            .child(
                div()
                    .text_size(theme::size::small())
                    .text_color(t.muted)
                    .child("brush"),
            )
            .children([1u32, 2, 4, 8].map(|s| {
                button(
                    format!("brush-{s}").into(),
                    s.to_string().into(),
                    self.retouch.size == s,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.retouch.size = s;
                    cx.notify();
                }))
            }));
        let color = self.retouch.color;
        let info: SharedString = match (self.retouch.tool, color) {
            (Tool::Pencil, None) => "click a palette color below to paint with it".into(),
            (Tool::Pencil, Some(c)) => format!("painting {}", crate::strip::hex(c)).into(),
            (Tool::Picker, _) => "click the image to take a color".into(),
            (Tool::NoDither | Tool::Dither, _) => "paints where dithering applies".into(),
            _ => "".into(),
        };
        let stale = self.retouch.layer.as_ref().is_some_and(|l| {
            !l.is_empty()
                && self
                    .output
                    .as_ref()
                    .is_some_and(|o| scale_of(l.size(), (o.width(), o.height())).is_none())
        });
        let mut v = vec![
            div()
                .mt_3()
                .text_size(theme::size::title())
                .text_color(t.text)
                .child("Retouch")
                .into_any_element(),
            tools.into_any_element(),
            sizes.into_any_element(),
        ];
        if !info.is_empty() {
            v.push(
                div()
                    .text_size(theme::size::small())
                    .text_color(t.muted)
                    .child(info)
                    .into_any_element(),
            );
        }
        if let Some((w, h)) = stale
            .then(|| self.retouch.layer.as_ref().map(Layer::size))
            .flatten()
        {
            v.push(
                div()
                    .text_size(theme::size::small())
                    .text_color(t.muted)
                    .child(format!("retouches hidden: they were painted at {w} x {h}"))
                    .into_any_element(),
            );
        }
        v.push(
            div()
                .flex()
                .flex_wrap()
                .gap_1()
                .child(
                    button("retouch-clear".into(), "clear retouches".into(), false).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.retouch.clear();
                            cx.notify();
                        }),
                    ),
                )
                .child(
                    button("mask-clear".into(), "clear mask".into(), false).on_click(cx.listener(
                        |this, _, _, cx| {
                            if this.settings.mask.take().is_some() {
                                this.settings_changed(false, cx);
                            }
                        },
                    )),
                )
                .child(
                    button("project-save".into(), "save with image".into(), false)
                        .on_click(cx.listener(|this, _, _, cx| this.save_project(cx))),
                )
                .into_any_element(),
        );
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_covers_both_ends_without_gaps() {
        let l = line((0, 0), (5, 2));
        assert_eq!(l.first(), Some(&(0, 0)));
        assert_eq!(l.last(), Some(&(5, 2)));
        assert_eq!(l.len(), 6);
        for w in l.windows(2) {
            assert!((w[1].0 - w[0].0).abs() <= 1 && (w[1].1 - w[0].1).abs() <= 1);
        }
        assert_eq!(line((3, 3), (3, 3)), vec![(3, 3)]);
        assert_eq!(line((2, 0), (0, 0)), vec![(2, 0), (1, 0), (0, 0)]);
    }

    #[test]
    fn strokes_undo_and_redo_exactly() {
        let mut r = Retouch {
            tool: Tool::Pencil,
            color: Some([255, 0, 0]),
            ..Retouch::default()
        };
        r.begin((0, 0), (4, 4));
        r.extend((3, 0));
        r.end();
        let painted = r.layer.clone().unwrap();
        assert_eq!(painted.px.iter().filter(|&&p| p != 0).count(), 4);
        r.tool = Tool::Eraser;
        r.begin((1, 0), (4, 4));
        r.end();
        assert_eq!(r.layer.as_ref().unwrap().px[1], 0);
        assert!(r.undo());
        assert_eq!(r.layer.as_ref(), Some(&painted));
        assert!(r.undo());
        assert!(r.layer.as_ref().unwrap().is_empty());
        assert!(!r.undo());
        assert!(r.redo());
        assert_eq!(r.layer.as_ref(), Some(&painted));
    }

    #[test]
    fn painting_needs_a_color_and_keeps_other_sizes() {
        let mut r = Retouch {
            tool: Tool::Pencil,
            ..Retouch::default()
        };
        r.begin((0, 0), (2, 2));
        r.end();
        assert!(r.layer.as_ref().unwrap().is_empty());
        r.color = Some([1, 2, 3]);
        r.begin((0, 0), (2, 2));
        r.end();
        // A painted layer is not thrown away for an output of another size.
        r.begin((0, 0), (3, 3));
        assert!(!r.drawing());
        assert_eq!(r.layer.as_ref().unwrap().size(), (2, 2));
    }

    #[test]
    fn compose_snaps_to_the_palette_and_skips_other_sizes() {
        let out = Image::new(2, 1, [0, 0, 0]).unwrap();
        let mut l = Layer::new(2, 1);
        l.put(1, 0, Some([250, 10, 10]));
        let img = compose(&out, &l, &[[0, 0, 0], [255, 0, 0]]).unwrap();
        assert_eq!(img.get(1, 0), Some([255, 0, 0, 255]));
        assert_eq!(img.get(0, 0), Some([0, 0, 0, 255]));
        assert!(compose(&Image::new(3, 1, [0, 0, 0]).unwrap(), &l, &[]).is_none());
        // At an export scale of 2 every painted pixel covers 2 x 2.
        let big = compose(&Image::new(4, 2, [0, 0, 0]).unwrap(), &l, &[[255, 0, 0]]).unwrap();
        assert_eq!(big.get(2, 1), Some([255, 0, 0, 255]));
        assert_eq!(big.get(3, 0), Some([255, 0, 0, 255]));
        assert_eq!(big.get(1, 1), Some([0, 0, 0, 255]));
        assert_eq!(scale_of((2, 1), (4, 3)), None);
        assert!(compose(&out, &Layer::new(2, 1), &[]).is_none());
    }

    #[test]
    fn layer_survives_png_round_trip() {
        let mut l = Layer::new(3, 2);
        l.put(0, 0, Some([1, 2, 3]));
        l.put(2, 1, Some([200, 100, 0]));
        let bytes =
            xd_art_core::io::encode(&l.to_image().unwrap(), xd_art_core::io::Format::Png).unwrap();
        let back = xd_art_core::io::decode_with(&bytes, &Default::default())
            .unwrap()
            .image;
        assert_eq!(Layer::from_image(&back), l);
    }

    #[test]
    fn mask_brush_scales_to_the_mask_size() {
        // A new mask is white at the output size.
        let m = paint_mask(None, (4, 4), &[(0, 0)], 1, 0).unwrap();
        assert_eq!(m.size(), (4, 4));
        assert_eq!(m.data().iter().filter(|&&v| v == 0).count(), 1);
        // A loaded mask twice as large gets 2 x 2 mask pixels per output pixel.
        let big = Mask::new(8, 8, vec![255; 64]).unwrap();
        let m = paint_mask(Some(&big), (4, 4), &[(3, 3)], 1, 0).unwrap();
        assert_eq!(m.data().iter().filter(|&&v| v == 0).count(), 4);
        assert_eq!(m.data()[63], 0);
        // A brush of 2 covers 4 output pixels; off-image parts are ignored.
        let m = paint_mask(None, (4, 4), &[(1, 1)], 2, 0).unwrap();
        assert_eq!(m.data().iter().filter(|&&v| v == 0).count(), 4);
        let m = paint_mask(None, (4, 4), &[(0, 0)], 4, 0).unwrap();
        assert_eq!(m.data().iter().filter(|&&v| v == 0).count(), 9);
    }

    #[test]
    fn sidecar_sits_next_to_the_source() {
        let p = sidecar(Path::new("art/cat.png"), "mask");
        assert_eq!(p, Path::new("art/cat.png.mask.png"));
    }
}
