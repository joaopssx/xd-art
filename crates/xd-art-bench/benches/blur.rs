//! Blur speed: exact gaussian, running-sum approximation and the original's box blur.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use xd_art_core::cancel::Cancel;
use xd_art_core::pool::Pool;
use xd_art_core::prep;

fn blur(c: &mut Criterion) {
    let src = xd_art_bench::photo_like(1920, 1080, 7).to_work();
    let pool = Pool::new();
    let cancel = Cancel::new();
    let mut g = c.benchmark_group("blur 1920x1080");
    g.sample_size(10);
    for radius in [2.0f32, 8.0, 32.0] {
        let mut img = src.clone();
        g.bench_with_input(BenchmarkId::new("gaussian", radius), &radius, |b, &r| {
            b.iter(|| prep::gaussian(&mut img, r, &pool, &cancel).expect("not cancelled"));
        });
        g.bench_with_input(BenchmarkId::new("running box", radius), &radius, |b, &r| {
            b.iter(|| prep::fast_gaussian(&mut img, r, &pool, &cancel).expect("not cancelled"));
        });
        g.bench_with_input(
            BenchmarkId::new("original box", radius),
            &radius,
            |b, &r| {
                b.iter(|| prep::box_blur(&mut img, r, &pool, &cancel).expect("not cancelled"));
            },
        );
    }
    g.finish();
}

criterion_group!(benches, blur);
criterion_main!(benches);
