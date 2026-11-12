//! Shared test code: synthetic fixtures, golden files and pixel comparison.

#![allow(dead_code)] // each test binary uses a different part of this

use std::fmt;
use std::path::PathBuf;

use xd_art_core::buffer::{Image, Work};

/// Same xorshift as `scripts/golden-harness.c`.
pub fn xorshift(s: &mut u32) -> u32 {
    *s ^= *s << 13;
    *s ^= *s >> 17;
    *s ^= *s << 5;
    *s
}

/// 48x40 gradient, red across, green down, blue on the diagonal.
pub fn gradient() -> Image {
    let mut img = Image::new(48, 40, [0; 3]).unwrap();
    for y in 0..40u32 {
        for x in 0..48u32 {
            img.set(
                x,
                y,
                [
                    (x * 255 / 47) as u8,
                    (y * 255 / 39) as u8,
                    ((x + y) * 7) as u8,
                    255,
                ],
            );
        }
    }
    img
}

/// 48x40 noise, with an alpha ramp in the 8 leftmost columns.
pub fn noise() -> Image {
    let mut img = Image::new(48, 40, [0; 3]).unwrap();
    let mut s = 1;
    for y in 0..40u32 {
        for x in 0..48u32 {
            let v = xorshift(&mut s);
            let a = if x < 8 { (y * 6) as u8 } else { 255 };
            img.set(x, y, [v as u8, (v >> 8) as u8, (v >> 16) as u8, a]);
        }
    }
    img
}

/// Noise of any size with a seed, for property and speed tests.
pub fn noise_sized(w: u32, h: u32, seed: u32) -> Image {
    let mut img = Image::new(w, h, [0; 3]).unwrap();
    let mut s = seed.max(1);
    for y in 0..h {
        for x in 0..w {
            let v = xorshift(&mut s);
            img.set(x, y, [v as u8, (v >> 8) as u8, (v >> 16) as u8, 255]);
        }
    }
    img
}

/// PICO-8 palette, as used by the golden harness.
pub const PICO8: [[u8; 3]; 16] = [
    [0x00, 0x00, 0x00],
    [0x1d, 0x2b, 0x53],
    [0x7e, 0x25, 0x53],
    [0x00, 0x87, 0x51],
    [0xab, 0x52, 0x36],
    [0x5f, 0x57, 0x4f],
    [0xc2, 0xc3, 0xc7],
    [0xff, 0xf1, 0xe8],
    [0xff, 0x00, 0x4d],
    [0xff, 0xa3, 0x00],
    [0xff, 0xec, 0x27],
    [0x00, 0xe4, 0x36],
    [0x29, 0xad, 0xff],
    [0x83, 0x76, 0x9c],
    [0xff, 0x77, 0xa8],
    [0xff, 0xcc, 0xaa],
];

/// Path of a golden file.
pub fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.rgba"))
}

/// Reads a golden file as `(width, height, rgba)`.
pub fn golden(name: &str) -> (u32, u32, Vec<u8>) {
    let path = golden_path(name);
    let b = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&b[..4], b"XDG1", "{name}: bad magic");
    let w = u32::from_le_bytes(b[4..8].try_into().unwrap());
    let h = u32::from_le_bytes(b[8..12].try_into().unwrap());
    assert_eq!(b.len(), 12 + (w * h * 4) as usize, "{name}: bad length");
    (w, h, b[12..].to_vec())
}

/// RGBA bytes of a working image, rounded down to 8 bits like the original's output.
pub fn work_rgba(w: &Work) -> Vec<u8> {
    w.px.iter()
        .flat_map(|p| p.map(|c| (c >> 7) as u8))
        .collect()
}

/// How two images differ.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Diff {
    /// Channels that differ by more than the tolerance.
    pub count: usize,
    /// Largest channel difference.
    pub max: u8,
    /// First pixel over the tolerance, as `(x, y, expected, actual)`.
    pub first: Option<(u32, u32, [u8; 4], [u8; 4])>,
    /// Pixels compared.
    pub pixels: usize,
}

impl fmt::Display for Diff {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} of {} channels off, max diff {}",
            self.count,
            self.pixels * 4,
            self.max
        )?;
        if let Some((x, y, e, a)) = self.first {
            write!(f, ", first at ({x}, {y}): expected {e:?}, got {a:?}")?;
        }
        Ok(())
    }
}

/// Compares RGBA buffers of a `width` wide image channel by channel.
pub fn diff(width: u32, expected: &[u8], actual: &[u8], tolerance: u8) -> Diff {
    assert_eq!(expected.len(), actual.len(), "sizes differ");
    let mut d = Diff {
        pixels: expected.len() / 4,
        ..Diff::default()
    };
    for (i, (e, a)) in expected
        .chunks_exact(4)
        .zip(actual.chunks_exact(4))
        .enumerate()
    {
        let mut off = false;
        for c in 0..4 {
            let delta = e[c].abs_diff(a[c]);
            d.max = d.max.max(delta);
            if delta > tolerance {
                d.count += 1;
                off = true;
            }
        }
        if off && d.first.is_none() {
            let (x, y) = (i as u32 % width, i as u32 / width);
            d.first = Some((x, y, e.try_into().unwrap(), a.try_into().unwrap()));
        }
    }
    d
}

/// Panics with a readable report when `actual` is not within `tolerance` of the golden.
pub fn assert_golden(name: &str, width: u32, height: u32, actual: &[u8], tolerance: u8) {
    let (gw, gh, expected) = golden(name);
    assert_eq!(
        (gw, gh),
        (width, height),
        "{name}: size differs from golden"
    );
    let d = diff(width, &expected, actual, tolerance);
    assert_eq!(d.count, 0, "{name}: {d}");
}
