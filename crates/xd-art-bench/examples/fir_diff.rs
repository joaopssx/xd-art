//! How far `fast_image_resize` is from xd-art's (and the original's) sampling.
//!
//! `cargo run --release -p xd-art-bench --example fir_diff`
//!
//! Prints, per filter, the largest channel difference and the share of channels that
//! differ by more than one step. This is what decided whether the crate could replace
//! the ported filters; see docs/decisions.md.

use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer, images};
use xd_art_core::cancel::Cancel;
use xd_art_core::pool::Pool;
use xd_art_core::sampling;
use xd_art_core::settings::SampleMode;

fn main() {
    let src = xd_art_bench::photo_like(1920, 1080, 7);
    let work = src.to_work();
    let fir_src = images::Image::from_vec_u8(1920, 1080, src.rgb().to_vec(), PixelType::U8x3)
        .expect("buffer matches size");
    let mut resizer = Resizer::new();
    for (mode, f) in [
        (SampleMode::Bilinear, FilterType::Bilinear),
        (SampleMode::Bicubic, FilterType::CatmullRom),
        (SampleMode::Lanczos, FilterType::Lanczos3),
        (SampleMode::Box, FilterType::Box),
    ] {
        for (w, h) in [(160, 90), (960, 540)] {
            let ours =
                sampling::sample(&work, w, h, mode, (0.0, 0.0), &Pool::new(), &Cancel::new())
                    .expect("sampling works")
                    .to_image()
                    .expect("not empty");
            let mut dst = images::Image::new(w, h, PixelType::U8x3);
            let opts = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(f));
            resizer
                .resize(&fir_src, &mut dst, &opts)
                .expect("resize works");
            let (mut max, mut off) = (0u8, 0usize);
            for (a, b) in ours.rgb().iter().zip(dst.buffer()) {
                let d = a.abs_diff(*b);
                max = max.max(d);
                off += usize::from(d > 1);
            }
            let share = off as f64 * 100.0 / ours.rgb().len() as f64;
            println!(
                "{:>8} {w}x{h}: max diff {max:3}, {share:5.1}% of channels off by more than 1",
                mode.name()
            );
        }
    }
}
