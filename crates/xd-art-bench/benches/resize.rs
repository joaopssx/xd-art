//! Sampling speed, and `fast_image_resize` on the same input for comparison.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer, images};
use xd_art_core::cancel::Cancel;
use xd_art_core::pool::Pool;
use xd_art_core::sampling;
use xd_art_core::settings::SampleMode;

fn resize(c: &mut Criterion) {
    let src = xd_art_bench::photo_like(1920, 1080, 7);
    let work = src.to_work();
    let pool = Pool::new();
    let cancel = Cancel::new();
    let mut g = c.benchmark_group("sample 1920x1080 to 160x90");
    for mode in SampleMode::ALL {
        g.bench_with_input(
            BenchmarkId::from_parameter(mode.name()),
            &mode,
            |b, &mode| {
                b.iter(|| {
                    let out = sampling::sample(&work, 160, 90, mode, (0.0, 0.0), &pool, &cancel)
                        .expect("sampling works");
                    pool.recycle(out);
                });
            },
        );
    }
    g.finish();

    let mut g = c.benchmark_group("sample 1920x1080 to 960x540");
    for mode in [SampleMode::Bilinear, SampleMode::Lanczos] {
        g.bench_with_input(
            BenchmarkId::from_parameter(mode.name()),
            &mode,
            |b, &mode| {
                b.iter(|| {
                    let out = sampling::sample(&work, 960, 540, mode, (0.0, 0.0), &pool, &cancel)
                        .expect("sampling works");
                    pool.recycle(out);
                });
            },
        );
    }
    g.finish();

    let fir_src = images::Image::from_vec_u8(1920, 1080, src.rgb().to_vec(), PixelType::U8x3)
        .expect("buffer matches size");
    let mut resizer = Resizer::new();
    let mut g = c.benchmark_group("fast_image_resize 1920x1080 to 160x90");
    for (name, f) in [
        ("bilinear", FilterType::Bilinear),
        ("bicubic", FilterType::CatmullRom),
        ("lanczos", FilterType::Lanczos3),
        ("box", FilterType::Box),
    ] {
        let opts = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(f));
        g.bench_function(name, |b| {
            let mut dst = images::Image::new(160, 90, PixelType::U8x3);
            b.iter(|| {
                resizer
                    .resize(&fir_src, &mut dst, &opts)
                    .expect("resize works")
            });
        });
    }
    g.finish();
}

criterion_group!(benches, resize);
criterion_main!(benches);
