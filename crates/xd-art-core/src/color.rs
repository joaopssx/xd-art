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
    let lin = |v: f32| {
        if v > 0.04045 {
            ((v + 0.055) / 1.055).powf(2.4)
        } else {
            v / 12.92
        }
    };
    let [r, g, b] = to_rgb(c).map(lin);
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

/// A palette prepared for one metric: every color converted once.
#[derive(Clone, Debug)]
pub struct Matcher {
    metric: Metric,
    colors: Vec<[u8; 3]>,
    coords: Vec<Coords>,
}

impl Matcher {
    /// Prepares `colors` for `metric`.
    #[must_use]
    pub fn new(colors: &[[u8; 3]], metric: Metric) -> Self {
        Self {
            metric,
            colors: colors.to_vec(),
            coords: colors.iter().map(|&c| coords(metric, c)).collect(),
        }
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
    #[must_use]
    pub fn closest(&self, c: [u8; 3]) -> u8 {
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

    #[test]
    fn preview_metric_is_cheap() {
        for m in Metric::ALL {
            assert!(m.preview().is_cheap());
        }
        assert_eq!(Metric::Weighted.preview(), Metric::Weighted);
    }
}
