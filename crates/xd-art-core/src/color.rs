//! Color spaces, distance metrics and closest color search.
//!
//! Everything here is the original program's math, including its choices: RGB
//! metrics work on values divided by 255, LAB uses the sRGB curve and the D65 matrix but
//! skips the white point normalization, and CIEDE2000 mixes `f32` and `f64` the way the
//! C code does (several constants are `float` literals promoted to `double`). Matching
//! that is what keeps palette mapping identical to the original's.

use std::f32::consts::PI;

use crate::settings::Metric;

/// A color converted to the space a metric works in.
pub type Coords = [f32; 3];

/// RGB scaled to `0..=1`.
#[inline]
#[must_use]
pub fn to_rgb(c: [u8; 3]) -> Coords {
    c.map(|v| f32::from(v) / 255.0)
}

/// CIELAB, the original's way.
#[must_use]
pub fn to_lab(c: [u8; 3]) -> Coords {
    // The sRGB curve for every 8-bit value, computed by build.rs with the same formula.
    let [r, g, b] = c.map(|v| crate::tables::SRGB_TO_LINEAR[v as usize]);
    let x = r * 0.4124 + g * 0.3576 + b * 0.1805;
    let y = r * 0.2126 + g * 0.7152 + b * 0.0722;
    let z = r * 0.0193 + g * 0.1192 + b * 0.9504;
    let f = |t: f32| {
        if t > 0.008_856 {
            t.powf(1.0 / 3.0)
        } else {
            7.787 * t + 16.0 / 116.0
        }
    };
    let (x, y, z) = (f(x), f(y), f(z));
    [116.0 * y - 16.0, 500.0 * (x - y), 200.0 * (y - z)]
}

/// Converts to the space `metric` measures in.
#[must_use]
pub fn coords(metric: Metric, c: [u8; 3]) -> Coords {
    if uses_lab(metric) {
        to_lab(c)
    } else {
        to_rgb(c)
    }
}

/// Whether the metric works in LAB.
#[must_use]
pub fn uses_lab(metric: Metric) -> bool {
    matches!(metric, Metric::Cie76 | Metric::Cie94 | Metric::Ciede2000)
}

#[inline]
fn sq3(a: Coords, b: Coords) -> [f32; 3] {
    [
        (b[0] - a[0]) * (b[0] - a[0]),
        (b[1] - a[1]) * (b[1] - a[1]),
        (b[2] - a[2]) * (b[2] - a[2]),
    ]
}

/// Distance between two colors already converted with [`coords`]. Smaller is closer.
/// Not a true distance for every metric (most are squared), only an ordering.
#[inline]
#[must_use]
pub fn distance(metric: Metric, a: Coords, b: Coords) -> f32 {
    match metric {
        Metric::Rgb | Metric::Cie76 => {
            let d = sq3(a, b);
            d[0] + d[1] + d[2]
        }
        Metric::Weighted => {
            let d = sq3(a, b);
            if a[0] + b[0] < 1.0 {
                d[0] * 2.0 + d[1] * 4.0 + d[2] * 3.0
            } else {
                d[0] * 3.0 + d[1] * 4.0 + d[2] * 2.0
            }
        }
        Metric::Redmean => {
            let r = 0.5 * 255.0 * (a[0] + b[0]);
            (2.0 + r / 256.0) * (b[0] - a[0]) * (b[0] - a[0])
                + 4.0 * (b[1] - a[1]) * (b[1] - a[1])
                + (2.0 + (255.0 - r) / 256.0) * (b[2] - a[2]) * (b[2] - a[2])
        }
        Metric::Cie94 => cie94(a, b),
        Metric::Ciede2000 => ciede2000(a, b),
    }
}

fn cie94(a: Coords, b: Coords) -> f32 {
    let l = a[0] - b[0];
    let c1 = (a[1] * a[1] + a[2] * a[2]).sqrt();
    let c2 = (b[1] * b[1] + b[2] * b[2]).sqrt();
    let c = c1 - c2;
    let h = ((a[1] - b[1]) * (a[1] - b[1]) + (a[2] - b[2]) * (a[2] - b[2]) - c * c).sqrt();
    let r2 = c / (1.0 + 0.045 * c1);
    let r3 = h / (1.0 + 0.015 * c1);
    let d = l * l + r2 * r2 + r3 * r3;
    if d.is_nan() { 1e15 } else { d }
}

/// `DEG2RAD` of the original: computed in `f32`, used as `f64`.
fn deg(a: f32) -> f64 {
    f64::from(a * PI / 180.0)
}

// Written as in the original, `(a + b) / 2.0` and `-1.0 * x` included, so every
// rounding step is the same.
#[allow(clippy::manual_midpoint, clippy::neg_multiply)]
fn ciede2000(b: Coords, c: Coords) -> f32 {
    // `x * x + y * y` of two floats is a float expression in the C code; it is only
    // promoted to double when passed to `sqrt`.
    let pi = f64::from(PI);
    let two_pi = f64::from(2.0 * PI);
    let k7 = f64::from(6_103_515_625.0f32);
    let cb1 = f64::from(b[1] * b[1] + b[2] * b[2]).sqrt();
    let cc1 = f64::from(c[1] * c[1] + c[2] * c[2]).sqrt();
    let c_bar = (cb1 + cc1) / 2.0;
    let c7 = c_bar.powf(7.0);
    let v = 0.5 * (1.0 - (c7 / (c7 + k7)).sqrt());
    let a1 = (1.0 + v) * f64::from(b[1]);
    let a2 = (1.0 + v) * f64::from(c[1]);
    let cs1 = (a1 * a1 + f64::from(b[2] * b[2])).sqrt();
    let cs2 = (a2 * a2 + f64::from(c[2] * c[2])).sqrt();
    let hue = |y: f32, x: f64| {
        if y != 0.0 || x != 0.0 {
            let h = f64::from(y).atan2(x);
            if h < 0.0 { h + two_pi } else { h }
        } else {
            0.0
        }
    };
    let h1 = hue(b[2], a1);
    let h2 = hue(c[2], a2);
    let dl = f64::from(c[0] - b[0]);
    let dc = cs2 - cs1;
    let mut dh = 0.0;
    if cs1 * cs2 != 0.0 {
        dh = h2 - h1;
        if dh < -pi {
            dh += two_pi;
        } else if dh > pi {
            dh -= two_pi;
        }
    }
    let big_h = 2.0 * (cs1 * cs2).sqrt() * (dh / 2.0).sin();
    let cs_bar = (cs1 + cs2) / 2.0;
    let mut h_bar = h1 + h2;
    if cs1 * cs2 != 0.0 {
        h_bar = if (h1 - h2).abs() <= pi {
            (h1 + h2) / 2.0
        } else if h1 + h2 < two_pi {
            (h1 + h2 + two_pi) / 2.0
        } else {
            (h1 + h2 - two_pi) / 2.0
        };
    }
    let f = |v: f32| f64::from(v);
    let t = 1.0 - f(0.17) * (h_bar - deg(30.0)).cos()
        + f(0.24) * (2.0 * h_bar).cos()
        + f(0.32) * (3.0 * h_bar + deg(6.0)).cos()
        - f(0.2) * (4.0 * h_bar - deg(63.0)).cos();
    let e = (h_bar - deg(275.0)) / deg(25.0);
    let v = deg(60.0) * (-1.0 * e * e).exp();
    let cs7 = cs_bar.powf(7.0);
    let rc = 2.0 * (cs7 / (cs7 + k7)).sqrt();
    let rt = -1.0 * v.sin() * rc;
    let sc = 1.0 + f(0.045) * cs_bar;
    let sh = 1.0 + f(0.015) * cs_bar * t;
    // The rotation term uses the mean hue where the formula has the hue difference.
    // Kept: it changes which color is picked for some inputs, see docs/compat.md.
    (dl * dl + (dc / sc) * (dc / sc) + (big_h / sh) * (big_h / sh) + rt * (dc / sc) * (h_bar / sh))
        as f32
}

/// Palettes with at least this many colors get a kd-tree, when the metric allows it.
/// Below it the linear search wins; the crossover is measured in `docs/performance.md`.
pub const KD_FROM: usize = 64;

/// Smallest per channel weight of a metric, so that `weight * d * d` is a lower bound of
/// the distance to anything on the far side of a splitting plane `d` away. `None` for
/// the metrics that have no such bound (CIE94 and CIEDE2000).
fn axis_weights(metric: Metric) -> Option<[f32; 3]> {
    match metric {
        Metric::Rgb | Metric::Cie76 => Some([1.0; 3]),
        // Weighted: red is 2 or 3, green 4, blue 3 or 2. Redmean: red 2 + r/256 with r
        // in 0..255, green 4, blue 2 + (255 - r)/256. Same minimums for both.
        Metric::Weighted | Metric::Redmean => Some([2.0, 4.0, 2.0]),
        Metric::Cie94 | Metric::Ciede2000 => None,
    }
}

#[derive(Clone, Copy, Debug)]
struct Node {
    point: Coords,
    index: u8,
    axis: u8,
    left: u16,
    right: u16,
}

const NONE: u16 = u16::MAX;

/// A kd-tree over the palette in the metric's coordinates. Queries give exactly the
/// linear search's answer, ties included: the distance is the same function, pruning
/// only skips subtrees whose lower bound is clearly larger than the best so far, and
/// equal distances prefer the lower palette index.
#[derive(Clone, Debug)]
struct KdTree {
    nodes: Vec<Node>,
    root: u16,
    weights: [f32; 3],
}

impl KdTree {
    fn new(coords: &[Coords], weights: [f32; 3]) -> Self {
        let mut idx: Vec<u8> = (0..coords.len()).map(|i| i as u8).collect();
        let mut nodes = Vec::with_capacity(coords.len());
        let root = Self::build(coords, &mut idx, 0, &mut nodes);
        Self {
            nodes,
            root,
            weights,
        }
    }

    fn build(coords: &[Coords], idx: &mut [u8], depth: usize, nodes: &mut Vec<Node>) -> u16 {
        if idx.is_empty() {
            return NONE;
        }
        // Split on the axis with the widest spread, which beats round robin for
        // palettes that are mostly one hue.
        let spread = |a: usize| {
            let (lo, hi) = idx.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &i| {
                let v = coords[i as usize][a];
                (lo.min(v), hi.max(v))
            });
            hi - lo
        };
        let axis = (0..3)
            .max_by(|&a, &b| spread(a).total_cmp(&spread(b)))
            .unwrap_or(depth % 3);
        idx.sort_by(|&a, &b| {
            coords[a as usize][axis]
                .total_cmp(&coords[b as usize][axis])
                .then(a.cmp(&b))
        });
        let mid = idx.len() / 2;
        let index = idx[mid];
        let at = nodes.len();
        nodes.push(Node {
            point: coords[index as usize],
            index,
            axis: axis as u8,
            left: NONE,
            right: NONE,
        });
        let (lo, hi) = idx.split_at_mut(mid);
        let left = Self::build(coords, lo, depth + 1, nodes);
        let right = Self::build(coords, &mut hi[1..], depth + 1, nodes);
        nodes[at].left = left;
        nodes[at].right = right;
        at as u16
    }

    fn closest(&self, metric: Metric, p: Coords) -> u8 {
        let mut best = (f32::MAX, u8::MAX);
        // (node, lower bound of anything below it). Depth is at most 9 for 256 colors.
        let mut stack = [(NONE, 0.0f32); 32];
        stack[0] = (self.root, 0.0);
        let mut top = 1;
        while top > 0 {
            top -= 1;
            let (n, bound) = stack[top];
            if n == NONE || bound > best.0 {
                continue;
            }
            let node = &self.nodes[n as usize];
            let d = distance(metric, p, node.point);
            if d < best.0 || (d == best.0 && node.index < best.1) {
                best = (d, node.index);
            }
            let a = node.axis as usize;
            let diff = p[a] - node.point[a];
            let (near, far) = if diff < 0.0 {
                (node.left, node.right)
            } else {
                (node.right, node.left)
            };
            // A hair under the true bound, so f32 rounding can never prune the answer.
            let far_bound = bound.max(self.weights[a] * diff * diff * 0.999);
            stack[top] = (far, far_bound);
            stack[top + 1] = (near, bound);
            top += 2;
        }
        best.1
    }
}

/// A palette prepared for one metric: every color converted once.
#[derive(Clone, Debug)]
pub struct Matcher {
    metric: Metric,
    colors: Vec<[u8; 3]>,
    coords: Vec<Coords>,
    tree: Option<KdTree>,
}

impl Matcher {
    /// Prepares `colors` for `metric`. Builds a kd-tree for palettes of [`KD_FROM`]
    /// colors or more when the metric allows exact pruning.
    #[must_use]
    pub fn new(colors: &[[u8; 3]], metric: Metric) -> Self {
        let coords: Vec<Coords> = colors.iter().map(|&c| coords(metric, c)).collect();
        let tree = axis_weights(metric)
            .filter(|_| colors.len() >= KD_FROM)
            .map(|w| KdTree::new(&coords, w));
        Self {
            metric,
            colors: colors.to_vec(),
            coords,
            tree,
        }
    }

    /// Whether lookups go through a kd-tree.
    #[must_use]
    pub fn has_tree(&self) -> bool {
        self.tree.is_some()
    }

    /// The metric.
    #[must_use]
    pub fn metric(&self) -> Metric {
        self.metric
    }

    /// The palette colors.
    #[must_use]
    pub fn colors(&self) -> &[[u8; 3]] {
        &self.colors
    }

    /// Index of the closest palette color. Ties go to the lower index, as in the
    /// original's linear search.
    #[inline]
    #[must_use]
    pub fn closest(&self, c: [u8; 3]) -> u8 {
        match &self.tree {
            Some(t) => t.closest(self.metric, coords(self.metric, c)),
            None => self.closest_linear(c),
        }
    }

    /// [`Matcher::closest`] without the kd-tree. Same answer, kept for comparisons.
    #[must_use]
    pub fn closest_linear(&self, c: [u8; 3]) -> u8 {
        let p = coords(self.metric, c);
        let mut best = 1e12f32;
        let mut idx = 0u8;
        for (i, q) in self.coords.iter().enumerate() {
            let d = distance(self.metric, p, *q);
            if d < best {
                best = d;
                idx = i as u8;
            }
        }
        idx
    }

    /// The two closest palette colors, and where `c` falls on the line between them in
    /// RGB (`0` at the first, `1` at the second). Used by the picoCAD style dither.
    #[must_use]
    pub fn closest_two(&self, c: [u8; 3]) -> (u8, u8, f32) {
        let p = coords(self.metric, c);
        let (mut b0, mut b1) = (1e12f32, 1e12f32);
        let (mut i0, mut i1) = (0u8, 0u8);
        for (i, q) in self.coords.iter().enumerate() {
            let d = distance(self.metric, p, *q);
            if d < b0 {
                b1 = b0;
                i1 = i0;
                b0 = d;
                i0 = i as u8;
            } else if d < b1 {
                b1 = d;
                i1 = i as u8;
            }
        }
        let p0 = to_rgb(self.colors[i0 as usize]);
        let p1 = to_rgb(self.colors[i1 as usize]);
        let cc = to_rgb(c);
        let d = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let len2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
        let t = if len2 < 1e-6 {
            0.0
        } else {
            (((cc[0] - p0[0]) * d[0] + (cc[1] - p0[1]) * d[1] + (cc[2] - p0[2]) * d[2]) / len2)
                .clamp(0.0, 1.0)
        };
        (i0, i1, t)
    }
}

/// Slots in a [`Cache`]. 4096 entries of 4 bytes stay in L1/L2 cache, and pixel art
/// sources (already reduced, often flat) repeat colors a lot.
const CACHE_SLOTS: usize = 4096;

/// A small direct mapped cache in front of [`Matcher::closest`]. One per thread and per
/// run; it is only valid for the matcher it was used with, so a palette or metric change
/// means a new cache. Results are the same with or without it.
#[derive(Clone, Debug)]
pub struct Cache {
    /// `rgb << 8 | index`, or `u32::MAX` for an empty slot.
    slots: Vec<u32>,
    /// Lookups answered from the cache.
    pub hits: u64,
    /// Lookups that had to search the palette.
    pub misses: u64,
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            slots: vec![u32::MAX; CACHE_SLOTS],
            hits: 0,
            misses: 0,
        }
    }
}

impl Cache {
    /// [`Matcher::closest`], remembered.
    #[inline]
    pub fn closest(&mut self, m: &Matcher, c: [u8; 3]) -> u8 {
        let key = u32::from(c[0]) << 16 | u32::from(c[1]) << 8 | u32::from(c[2]);
        // Fibonacci hashing spreads neighbouring colors over the table.
        let slot = (key.wrapping_mul(0x9e37_79b1) >> 20) as usize & (CACHE_SLOTS - 1);
        let e = self.slots[slot];
        if e != u32::MAX && e >> 8 == key {
            self.hits += 1;
            return e as u8;
        }
        self.misses += 1;
        let i = m.closest(c);
        self.slots[slot] = key << 8 | u32::from(i);
        i
    }
}

/// Cells per channel in a [`Lut`].
const LUT_SIDE: usize = 32;
/// Most candidates kept per cell.
const LUT_CANDIDATES: usize = 8;

/// A 32x32x32 table over RGB for fast approximate mapping.
///
/// The pipeline does not use it: at the output sizes pixel art has, building the table
/// costs more than the exact search behind [`Cache`] (see `docs/performance.md`). It is
/// here for callers that map many more pixels with one palette.
///
/// Each cell keeps the palette colors that are closest to its center or to one of its
/// corners (up to eight). A lookup measures the exact distance to those candidates
/// only. That is exact whenever the true answer is among them, which it is for nearly
/// every color; the rate of misses is measured in `docs/performance.md`.
#[derive(Clone, Debug)]
pub struct Lut {
    candidates: Vec<[u8; LUT_CANDIDATES]>,
    counts: Vec<u8>,
}

impl Lut {
    /// Builds the table: 9 exact queries per cell, 294,912 in total.
    #[must_use]
    pub fn new(m: &Matcher) -> Self {
        let n = LUT_SIDE * LUT_SIDE * LUT_SIDE;
        let mut candidates = vec![[0u8; LUT_CANDIDATES]; n];
        let mut counts = vec![0u8; n];
        let step = 256 / LUT_SIDE;
        let corner = |i: usize, hi: bool| {
            if hi {
                (i * step + step - 1) as u8
            } else {
                (i * step) as u8
            }
        };
        for r in 0..LUT_SIDE {
            for g in 0..LUT_SIDE {
                for b in 0..LUT_SIDE {
                    let cell = (r * LUT_SIDE + g) * LUT_SIDE + b;
                    let (cand, count) = (&mut candidates[cell], &mut counts[cell]);
                    let mut push = |i: u8| {
                        if !cand[..*count as usize].contains(&i)
                            && (*count as usize) < LUT_CANDIDATES
                        {
                            cand[*count as usize] = i;
                            *count += 1;
                        }
                    };
                    let half = (step / 2) as u8;
                    push(m.closest([
                        corner(r, false) + half,
                        corner(g, false) + half,
                        corner(b, false) + half,
                    ]));
                    for k in 0..8 {
                        push(m.closest([
                            corner(r, k & 4 != 0),
                            corner(g, k & 2 != 0),
                            corner(b, k & 1 != 0),
                        ]));
                    }
                }
            }
        }
        Self { candidates, counts }
    }

    /// The closest palette color among the cell's candidates.
    #[inline]
    #[must_use]
    pub fn closest(&self, m: &Matcher, c: [u8; 3]) -> u8 {
        let s = 256 / LUT_SIDE;
        let cell = (usize::from(c[0]) / s * LUT_SIDE + usize::from(c[1]) / s) * LUT_SIDE
            + usize::from(c[2]) / s;
        let n = self.counts[cell] as usize;
        let cand = &self.candidates[cell][..n];
        if n == 1 {
            return cand[0];
        }
        let p = coords(m.metric, c);
        let mut best = 1e12f32;
        let mut idx = cand[0];
        // Candidates are tried in palette order so ties resolve like the full search.
        let mut order = [0u8; LUT_CANDIDATES];
        order[..n].copy_from_slice(cand);
        order[..n].sort_unstable();
        for &i in &order[..n] {
            let d = distance(m.metric, p, m.coords[i as usize]);
            if d < best {
                best = d;
                idx = i;
            }
        }
        idx
    }
}

impl Metric {
    /// The metric to use while a slider is being dragged: the LAB metrics cost several
    /// times more per comparison, so the preview falls back to redmean until the slider
    /// is released.
    #[must_use]
    pub fn preview(self) -> Self {
        if self.is_cheap() {
            self
        } else {
            Metric::Redmean
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lab_reference_points() {
        let white = to_lab([255, 255, 255]);
        assert!((white[0] - 100.0).abs() < 0.1, "{white:?}");
        let black = to_lab([0, 0, 0]);
        assert!(black[0].abs() < 1e-3);
        // No white point normalization, as in the original, so white lands at about
        // a = -8.5, b = -5.8 instead of 0, 0.
        assert!(
            (white[1] + 8.5).abs() < 0.2 && (white[2] + 5.8).abs() < 0.2,
            "{white:?}"
        );
    }

    #[test]
    fn identical_colors_have_zero_distance() {
        for m in Metric::ALL {
            let c = coords(m, [10, 200, 30]);
            assert!(distance(m, c, c).abs() < 1e-6, "{m:?}");
        }
    }

    #[test]
    fn closest_picks_exact_matches_and_breaks_ties_low() {
        let pal = [[0, 0, 0], [255, 0, 0], [0, 255, 0], [255, 0, 0]];
        for m in Metric::ALL {
            let mt = Matcher::new(&pal, m);
            assert_eq!(mt.closest([0, 255, 0]), 2, "{m:?}");
            assert_eq!(mt.closest([250, 5, 5]), 1, "{m:?}");
        }
    }

    #[test]
    fn closest_two_projects() {
        let mt = Matcher::new(&[[0, 0, 0], [255, 255, 255]], Metric::Rgb);
        let (a, b, t) = mt.closest_two([100, 100, 100]);
        assert_eq!((a, b), (0, 1));
        assert!((t - 100.0 / 255.0).abs() < 1e-5);
        let single = Matcher::new(&[[5, 5, 5]], Metric::Rgb);
        assert_eq!(single.closest_two([0, 0, 0]), (0, 0, 0.0));
    }

    fn pseudo_colors(n: usize) -> Vec<[u8; 3]> {
        let mut s = 12345u32;
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                [s as u8, (s >> 8) as u8, (s >> 16) as u8]
            })
            .collect()
    }

    #[test]
    fn cache_gives_the_same_answers() {
        let pal = pseudo_colors(40);
        for metric in Metric::ALL {
            let m = Matcher::new(&pal, metric);
            let mut c = Cache::default();
            for col in pseudo_colors(3000).iter().chain(pseudo_colors(3000).iter()) {
                assert_eq!(c.closest(&m, *col), m.closest(*col), "{metric:?}");
            }
            assert!(c.hits > 0 && c.misses > 0);
        }
    }

    #[test]
    fn lut_is_close_to_exact() {
        let pal = pseudo_colors(24);
        let m = Matcher::new(&pal, Metric::Redmean);
        let lut = Lut::new(&m);
        let colors = pseudo_colors(20_000);
        let wrong = colors
            .iter()
            .filter(|c| lut.closest(&m, **c) != m.closest(**c))
            .count();
        // Over all 2^24 colors the table disagrees with the exact search on less than
        // 0.005% for the built-in palettes (example `lut_error` in xd-art-bench).
        assert!(wrong * 1000 < colors.len(), "{wrong} of {}", colors.len());
        for &c in &pal {
            assert_eq!(pal[lut.closest(&m, c) as usize], c);
        }
    }

    #[test]
    fn kd_tree_matches_linear_search() {
        for n in [KD_FROM, 100, 256] {
            let pal = pseudo_colors(n);
            for metric in Metric::ALL {
                let m = Matcher::new(&pal, metric);
                assert_eq!(
                    m.has_tree(),
                    !matches!(metric, Metric::Cie94 | Metric::Ciede2000)
                );
                for c in pseudo_colors(5000).into_iter().chain(pal.iter().copied()) {
                    assert_eq!(m.closest(c), m.closest_linear(c), "{metric:?} {n} {c:?}");
                }
            }
        }
        // Duplicate colors: the lower index has to win, like in the linear search.
        let mut dup = pseudo_colors(40);
        dup[30] = dup[3];
        let m = Matcher::new(&dup, Metric::Rgb);
        assert_eq!(m.closest(dup[3]), 3);
    }

    #[test]
    fn preview_metric_is_cheap() {
        for m in Metric::ALL {
            assert!(m.preview().is_cheap());
        }
        assert_eq!(Metric::Weighted.preview(), Metric::Weighted);
    }
}
