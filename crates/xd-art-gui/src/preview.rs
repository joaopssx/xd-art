//! The image view: zoom, pan, pixel grid, and drawing with hard pixel edges.
//!
//! gpui's renderers sample images with linear filtering, which would blur pixel art at
//! any zoom that is not exactly 1. So the view never asks the GPU to scale: on every
//! change of zoom, position or size it builds, on the CPU, an image of exactly the
//! visible area at the window's device resolution, with every pixel picked by nearest
//! neighbour, and has gpui draw that at 1:1. Only the visible part is built, so the cost
//! follows the window size, not the image size.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Bounds, Corners, Pixels, Point, RenderImage, Window, canvas, point, prelude::*};
use image::{Frame, RgbaImage};
use xd_art_core::buffer::Image;

/// Zoom levels, in screen pixels per image pixel. Above 1 they are whole numbers, so
/// every image pixel is the same size on screen.
pub const LEVELS: [f32; 22] = [
    0.05, 0.1, 0.125, 0.2, 0.25, 0.333, 0.5, 0.667, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0, 12.0,
    16.0, 20.0, 24.0, 32.0, 48.0,
];

/// The grid is drawn from this many screen pixels per image pixel up.
pub const GRID_FROM: f32 = 8.0;

/// The next level up or down from `zoom`.
pub fn step(zoom: f32, up: bool) -> f32 {
    if up {
        LEVELS
            .iter()
            .copied()
            .find(|&l| l > zoom * 1.001)
            .unwrap_or(LEVELS[LEVELS.len() - 1])
    } else {
        LEVELS
            .iter()
            .rev()
            .copied()
            .find(|&l| l < zoom * 0.999)
            .unwrap_or(LEVELS[0])
    }
}

/// The largest zoom that shows the whole image in `bounds`, a whole number when that
/// is at least 1.
pub fn fit_zoom(image: (u32, u32), bounds: (f32, f32)) -> f32 {
    let z = (bounds.0 / image.0 as f32).min(bounds.1 / image.1 as f32);
    if z >= 1.0 { z.floor() } else { z }
}

/// The source pixel shown at each device pixel along one axis, or `None` off the image.
fn axis_map(len: usize, origin: f32, scale: f32, src_len: u32) -> Vec<Option<u32>> {
    (0..len)
        .map(|d| {
            let s = ((d as f32 + 0.5 - origin) / scale).floor();
            (s >= 0.0 && (s as u32) < src_len).then_some(s as u32)
        })
        .collect()
}

/// Background for transparent pixels: an 8 device pixel checker in two dark grays.
fn checker(x: usize, y: usize) -> [u8; 3] {
    if (x / 8 + y / 8).is_multiple_of(2) {
        [0x1e, 0x20, 0x28]
    } else {
        [0x16, 0x18, 0x1f]
    }
}

/// What to draw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame2 {
    /// Size of the area in device pixels.
    pub size: (usize, usize),
    /// Where image pixel (0, 0) starts, in device pixels from the top left of the area.
    pub origin: (f32, f32),
    /// Device pixels per image pixel.
    pub scale: f32,
    /// Grid strength `0..=1`, 0 for none.
    pub grid: f32,
    /// Color outside the image.
    pub background: [u8; 3],
    /// Device x left of which the "before" image is shown, if comparing.
    pub split: Option<f32>,
}

/// Where the source pixel for output pixel `(x, y)` is, when the source is shown in
/// place of the output for comparison.
fn before_pixel(before: &Image, after: (u32, u32), x: u32, y: u32, fx: f32, fy: f32) -> usize {
    let sx =
        (((x as f32 + fx) * before.width() as f32 / after.0 as f32) as u32).min(before.width() - 1);
    let sy = (((y as f32 + fy) * before.height() as f32 / after.1 as f32) as u32)
        .min(before.height() - 1);
    sy as usize * before.width() as usize + sx as usize
}

/// Fills `out` with the BGRA pixels of the area described by `f`. With `before` and a
/// split, the part left of the split shows `before` stretched over the same area, so
/// both sides line up whatever their sizes.
pub fn render(src: &Image, before: Option<&Image>, f: &Frame2, out: &mut Vec<u8>) {
    let (w, h) = f.size;
    out.clear();
    out.resize(w * h * 4, 255);
    let cols = axis_map(w, f.origin.0, f.scale, src.width());
    let rows = axis_map(h, f.origin.1, f.scale, src.height());
    let grid = f.grid > 0.0 && f.scale >= GRID_FROM;
    let edge =
        |d: usize, origin: f32| ((d as f32 - origin) / f.scale).fract().abs() * f.scale < 1.0;
    let sw = src.width() as usize;
    for (y, row) in rows.iter().enumerate() {
        let line = &mut out[y * w * 4..(y + 1) * w * 4];
        let ygrid = grid && edge(y, f.origin.1);
        for (x, col) in cols.iter().enumerate() {
            let px = &mut line[x * 4..x * 4 + 4];
            let left = f.split.is_some_and(|sp| (x as f32) < sp);
            if f.split.is_some_and(|sp| (x as f32 - sp).abs() < 1.0) {
                px.copy_from_slice(&[0xf6, 0x7c, 0x8b, 255]);
                continue;
            }
            let c = match (col, row) {
                (Some(sx), Some(sy)) => {
                    let (img, i) = match before.filter(|_| left) {
                        Some(b) => {
                            // Position inside the output pixel, so the source shows its
                            // own detail instead of blocks.
                            let fx = ((x as f32 + 0.5 - f.origin.0) / f.scale).fract();
                            let fy = ((y as f32 + 0.5 - f.origin.1) / f.scale).fract();
                            (
                                b,
                                before_pixel(b, (src.width(), src.height()), *sx, *sy, fx, fy),
                            )
                        }
                        None => (src, *sy as usize * sw + *sx as usize),
                    };
                    let (rgb, alpha) = (img.rgb(), img.alpha());
                    let p = [rgb[i * 3], rgb[i * 3 + 1], rgb[i * 3 + 2]];
                    let a = alpha.map_or(255, |a| a[i]);
                    let mut c = if a == 255 {
                        p
                    } else {
                        let bg = checker(x, y);
                        [0, 1, 2].map(|k| {
                            ((u32::from(p[k]) * u32::from(a)
                                + u32::from(bg[k]) * u32::from(255 - a))
                                / 255) as u8
                        })
                    };
                    if grid && !left && (ygrid || edge(x, f.origin.0)) {
                        let k = 1.0 - 0.6 * f.grid;
                        c = c.map(|v| (f32::from(v) * k) as u8);
                    }
                    c
                }
                _ => f.background,
            };
            px[0] = c[2];
            px[1] = c[1];
            px[2] = c[0];
            px[3] = 255;
        }
    }
}

#[derive(Default)]
struct Cache {
    key: Option<(usize, Frame2)>,
    image: Option<Arc<RenderImage>>,
    buffer: Vec<u8>,
    retired: Vec<Arc<RenderImage>>,
}

/// Zoom and position of the view, and the cached drawing.
pub struct Viewer {
    /// Screen (logical) pixels per image pixel.
    pub zoom: f32,
    /// Where image pixel (0, 0) is, in logical pixels from the top left of the view.
    pub origin: Point<f32>,
    /// Keep refitting to the view until the user zooms or pans.
    pub fitted: bool,
    /// Show the pixel grid when zoomed in far enough.
    pub grid: bool,
    /// Grid strength, `0..=1`.
    pub grid_strength: f32,
    /// Last mouse position while panning.
    pub panning: Option<Point<Pixels>>,
    /// Space is held: left drag pans.
    pub space: bool,
    /// Comparison: `None` off, `Some(t)` with the divider at fraction `t` of the width
    /// (1.0 shows only the source).
    pub compare: Option<f32>,
    /// The divider is being dragged.
    pub dragging_split: bool,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    /// Zoom and origin the last frame used while fitted, so the first zoom or pan after
    /// a fit starts from what was on screen.
    shown: Rc<Cell<(f32, Point<f32>)>>,
    cache: Rc<RefCell<Cache>>,
}

impl Default for Viewer {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            origin: point(0.0, 0.0),
            fitted: true,
            grid: true,
            grid_strength: 0.5,
            panning: None,
            space: false,
            compare: None,
            dragging_split: false,
            bounds: Rc::new(Cell::new(Bounds::default())),
            shown: Rc::new(Cell::new((1.0, point(0.0, 0.0)))),
            cache: Rc::new(RefCell::new(Cache::default())),
        }
    }
}

impl Viewer {
    /// The view's last known bounds in the window.
    pub fn bounds(&self) -> Bounds<Pixels> {
        self.bounds.get()
    }

    /// Fits the image into the view, now and whenever the view changes size, until the
    /// user zooms or pans.
    pub fn fit(&mut self) {
        self.fitted = true;
    }

    /// Shows the image at 100%, centered.
    pub fn actual_size(&mut self, image: (u32, u32)) {
        self.zoom = 1.0;
        self.center(image);
        self.fitted = false;
    }

    /// Centers the image at the current zoom.
    pub fn center(&mut self, image: (u32, u32)) {
        let b = self.bounds();
        self.origin = point(
            ((f32::from(b.size.width) - image.0 as f32 * self.zoom) / 2.0).round(),
            ((f32::from(b.size.height) - image.1 as f32 * self.zoom) / 2.0).round(),
        );
    }

    /// Takes over the zoom and position last drawn, before the user changes them.
    fn sync(&mut self) {
        if self.fitted {
            (self.zoom, self.origin) = self.shown.get();
        }
    }

    /// Zooms to `zoom`, keeping the image point under `anchor` (window position) still.
    pub fn zoom_at(&mut self, zoom: f32, anchor: Point<Pixels>) {
        self.sync();
        let b = self.bounds();
        let ax = f32::from(anchor.x - b.origin.x);
        let ay = f32::from(anchor.y - b.origin.y);
        let ix = (ax - self.origin.x) / self.zoom;
        let iy = (ay - self.origin.y) / self.zoom;
        self.zoom = zoom.clamp(LEVELS[0], LEVELS[LEVELS.len() - 1]);
        self.origin = point((ax - ix * self.zoom).round(), (ay - iy * self.zoom).round());
        self.fitted = false;
    }

    /// Zooms one level, around the middle of the view.
    pub fn zoom_step(&mut self, up: bool) {
        self.sync();
        let b = self.bounds();
        let mid = point(
            b.origin.x + b.size.width / 2.0,
            b.origin.y + b.size.height / 2.0,
        );
        self.zoom_at(step(self.zoom, up), mid);
    }

    /// Moves the image by a mouse delta.
    pub fn pan_by(&mut self, d: Point<Pixels>) {
        self.sync();
        self.origin = point(
            self.origin.x + f32::from(d.x),
            self.origin.y + f32::from(d.y),
        );
        self.fitted = false;
    }

    /// The element that draws `image`. `background` fills the area around it.
    /// Next comparison state: off, divider in the middle, source only, off.
    pub fn cycle_compare(&mut self) {
        self.compare = match self.compare {
            None => Some(0.5),
            Some(t) if t < 1.0 => Some(1.0),
            Some(_) => None,
        };
    }

    /// Moves the divider to the window x position `x`.
    pub fn split_at(&mut self, x: Pixels) {
        let b = self.bounds();
        let w = f32::from(b.size.width).max(1.0);
        self.compare = Some((f32::from(x - b.origin.x) / w).clamp(0.0, 1.0));
    }

    /// Whether `x` (window position) is on the divider.
    pub fn near_split(&self, x: Pixels) -> bool {
        let b = self.bounds();
        self.compare.is_some_and(|t| {
            t < 1.0 && (f32::from(x - b.origin.x) - t * f32::from(b.size.width)).abs() < 6.0
        })
    }

    /// The element that draws `image`, and `before` on the left of the divider when
    /// comparing. `background` fills the area around it.
    pub fn element(
        &self,
        image: Arc<Image>,
        before: Option<Arc<Image>>,
        background: [u8; 3],
    ) -> impl IntoElement {
        self.element_at(
            (self.zoom, self.origin, self.fitted),
            image,
            before,
            background,
        )
    }

    /// [`Viewer::element`] with the zoom, position and fit of another view, so several
    /// views can move together while each keeps its own cached drawing.
    pub fn element_at(
        &self,
        (zoom, origin, fitted): (f32, Point<f32>, bool),
        image: Arc<Image>,
        before: Option<Arc<Image>>,
        background: [u8; 3],
    ) -> impl IntoElement {
        let split = self.compare;
        let (bounds_cell, shown, cache) =
            (self.bounds.clone(), self.shown.clone(), self.cache.clone());
        let grid = if self.grid { self.grid_strength } else { 0.0 };
        canvas(
            move |bounds: Bounds<Pixels>, window: &mut Window, _: &mut App| {
                bounds_cell.set(bounds);
                let (bw, bh) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
                let (zoom, origin) = if fitted {
                    let size = (image.width(), image.height());
                    let z = fit_zoom(size, (bw - 16.0, bh - 16.0)).max(LEVELS[0]);
                    let o = point(
                        ((bw - size.0 as f32 * z) / 2.0).round(),
                        ((bh - size.1 as f32 * z) / 2.0).round(),
                    );
                    (z, o)
                } else {
                    (zoom, origin)
                };
                shown.set((zoom, origin));
                let s = window.scale_factor();
                let f = Frame2 {
                    size: (
                        (f32::from(bounds.size.width) * s).round() as usize,
                        (f32::from(bounds.size.height) * s).round() as usize,
                    ),
                    origin: (origin.x * s, origin.y * s),
                    scale: zoom * s,
                    grid,
                    background,
                    split: split
                        .filter(|_| before.is_some())
                        .map(|t| t * f32::from(bounds.size.width) * s),
                };
                let id = Arc::as_ptr(&image) as usize
                    ^ before
                        .as_ref()
                        .map_or(0, |b| Arc::as_ptr(b) as usize)
                        .rotate_left(17);
                let mut c = cache.borrow_mut();
                if f.size.0 == 0 || f.size.1 == 0 {
                    return None;
                }
                let retired = std::mem::take(&mut c.retired);
                if c.key != Some((id, f)) {
                    let mut buf = std::mem::take(&mut c.buffer);
                    render(&image, before.as_deref(), &f, &mut buf);
                    let frame = RgbaImage::from_raw(f.size.0 as u32, f.size.1 as u32, buf)?;
                    let fresh = Arc::new(RenderImage::new(vec![Frame::new(frame)]));
                    if let Some(old) = c.image.replace(fresh) {
                        c.retired.push(old);
                    }
                    c.key = Some((id, f));
                }
                c.image.clone().map(|img| (img, retired))
            },
            move |bounds, drawn, window: &mut Window, _| {
                let Some((img, retired)) = drawn else { return };
                // Images of earlier zoom levels are dropped here, otherwise gpui's
                // texture atlas would keep every one of them.
                for old in retired {
                    window.drop_image(old).ok();
                }
                window
                    .paint_image(bounds, Corners::default(), img, 0, false)
                    .ok();
            },
        )
        .size_full()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_step_and_fit() {
        assert_eq!(step(1.0, true), 2.0);
        assert_eq!(step(2.0, false), 1.0);
        assert_eq!(step(1.5, true), 2.0);
        assert_eq!(step(48.0, true), 48.0);
        assert_eq!(fit_zoom((64, 32), (400.0, 300.0)), 6.0);
        assert!((fit_zoom((2000, 1000), (400.0, 300.0)) - 0.2).abs() < 1e-6);
        for w in LEVELS.windows(2) {
            assert!(w[0] < w[1]);
        }
    }

    #[test]
    fn nearest_pixels_with_hard_edges() {
        let mut img = Image::new(2, 1, [255, 0, 0]).unwrap();
        img.set(1, 0, [0, 0, 255, 255]);
        let f = Frame2 {
            size: (6, 3),
            origin: (0.0, 0.0),
            scale: 3.0,
            grid: 0.0,
            background: [9, 9, 9],
            split: None,
        };
        let mut out = Vec::new();
        render(&img, None, &f, &mut out);
        let at = |x: usize, y: usize| {
            [
                out[(y * 6 + x) * 4 + 2],
                out[(y * 6 + x) * 4 + 1],
                out[(y * 6 + x) * 4],
            ]
        };
        assert_eq!(at(2, 0), [255, 0, 0]);
        assert_eq!(at(3, 0), [0, 0, 255]);
        assert_eq!(at(5, 2), [0, 0, 255]);
    }

    #[test]
    fn divider_shows_the_source_on_the_left() {
        let after = Image::new(2, 1, [10, 10, 10]).unwrap();
        let mut before = Image::new(4, 2, [200, 0, 0]).unwrap();
        before.set(3, 0, [0, 200, 0, 255]);
        let mut out = Vec::new();
        let at = |out: &[u8], x: usize, y: usize| {
            [
                out[(y * 8 + x) * 4 + 2],
                out[(y * 8 + x) * 4 + 1],
                out[(y * 8 + x) * 4],
            ]
        };
        let all_before = Frame2 {
            size: (8, 4),
            origin: (0.0, 0.0),
            scale: 4.0,
            grid: 0.0,
            background: [0; 3],
            split: Some(9.0),
        };
        render(&after, Some(&before), &all_before, &mut out);
        assert_eq!(at(&out, 0, 0), [200, 0, 0], "source on the left");
        // Device x 6 is inside output pixel 1, at the source's last column.
        assert_eq!(
            at(&out, 6, 0),
            [0, 200, 0],
            "source detail inside one output pixel"
        );
        let half = Frame2 {
            split: Some(2.0),
            ..all_before
        };
        render(&after, Some(&before), &half, &mut out);
        assert_eq!(at(&out, 7, 0), [10, 10, 10], "result on the right");
        assert_eq!(at(&out, 2, 2), [0x8b, 0x7c, 0xf6], "divider line");
    }

    #[test]
    fn outside_is_background_and_grid_needs_zoom() {
        let img = Image::new(1, 1, [200, 200, 200]).unwrap();
        let mut out = Vec::new();
        let f = Frame2 {
            size: (12, 12),
            origin: (2.0, 2.0),
            scale: 8.0,
            grid: 1.0,
            background: [1, 2, 3],
            split: None,
        };
        render(&img, None, &f, &mut out);
        assert_eq!(&out[0..3], &[3, 2, 1]);
        // (2, 2) is the first pixel of the image, on the grid line.
        let i = (2 * 12 + 2) * 4;
        assert!(out[i] < 200);
        // (5, 5) is inside the pixel, off the line.
        let j = (5 * 12 + 5) * 4;
        assert_eq!(out[j], 200);
        let low = Frame2 { scale: 4.0, ..f };
        render(&img, None, &low, &mut out);
        assert_eq!(out[i], 200, "no grid below GRID_FROM");
    }
}
