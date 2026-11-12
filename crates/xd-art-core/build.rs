//! Generates the constant tables of the engine into `$OUT_DIR/tables.rs`.
//!
//! - Bayer matrices of size 2, 4 and 8, built recursively. The 3x3 and 5x5 maps and the
//!   clustered dot maps are not recursive; their values are the original program's.
//! - The sRGB to linear curve for every 8-bit value.
//!
//! Thresholds are written as `k as f32 / n as f32` values, the same `f32` the original's
//! `k.0f / n.0f` literals produce.

use std::fmt::Write as _;
use std::path::PathBuf;

fn bayer(n: usize) -> Vec<u32> {
    if n == 1 {
        return vec![0];
    }
    let h = n / 2;
    let m = bayer(h);
    let mut out = vec![0; n * n];
    for y in 0..n {
        for x in 0..n {
            let base = 4 * m[(y % h) * h + x % h];
            out[y * n + x] = base
                + match (y / h, x / h) {
                    (0, 0) => 0,
                    (0, 1) => 2,
                    (1, 0) => 3,
                    _ => 1,
                };
        }
    }
    out
}

const BAYER3: [u32; 9] = [1, 3, 7, 8, 0, 4, 6, 5, 2];
const BAYER5: [u32; 25] = [
    18, 4, 12, 20, 1, 7, 19, 6, 15, 23, 21, 0, 13, 24, 3, 22, 11, 14, 8, 9, 5, 16, 2, 10, 17,
];
const CLUSTER4: [u32; 16] = [12, 5, 6, 13, 4, 0, 1, 7, 11, 3, 2, 8, 15, 10, 9, 14];
const CLUSTER8: [u32; 64] = [
    24, 10, 12, 26, 35, 47, 49, 37, 8, 0, 2, 14, 45, 59, 61, 51, 22, 6, 4, 16, 43, 57, 63, 53, 30,
    20, 18, 28, 33, 41, 55, 39, 34, 46, 48, 36, 25, 11, 13, 27, 44, 58, 60, 50, 9, 1, 3, 15, 42,
    56, 62, 52, 23, 7, 5, 17, 32, 40, 54, 38, 31, 21, 19, 29,
];

fn table(out: &mut String, name: &str, values: &[u32]) {
    let n = values.len();
    let _ = write!(out, "pub(crate) const {name}: [f32; {n}] = [");
    for v in values {
        let f = *v as f32 / n as f32;
        // `{:?}` prints the shortest text that parses back to the same f32.
        let _ = write!(out, "{f:?}, ");
    }
    out.push_str("];\n");
}

fn main() {
    let mut out = String::from("// Written by build.rs. Do not edit.\n");
    table(&mut out, "BAYER2", &bayer(2));
    table(&mut out, "BAYER4", &bayer(4));
    table(&mut out, "BAYER8", &bayer(8));
    table(&mut out, "BAYER3", &BAYER3);
    table(&mut out, "BAYER5", &BAYER5);
    table(&mut out, "CLUSTER4", &CLUSTER4);
    table(&mut out, "CLUSTER8", &CLUSTER8);
    out.push_str("pub(crate) const SRGB_TO_LINEAR: [f32; 256] = [");
    for v in 0..256u32 {
        let c = v as f32 / 255.0;
        let l = if c > 0.04045 {
            ((c + 0.055) / 1.055).powf(2.4)
        } else {
            c / 12.92
        };
        let _ = write!(out, "{l:?}, ");
    }
    out.push_str("];\n");
    let path =
        PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR")).join("tables.rs");
    std::fs::write(path, out).expect("OUT_DIR is writable");
    println!("cargo::rerun-if-changed=build.rs");
}
