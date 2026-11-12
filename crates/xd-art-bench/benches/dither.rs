//! Palette mapping speed per dither mode, on one thread and on all of them.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use xd_art_core::cancel::Cancel;
use xd_art_core::dither;
use xd_art_core::palette;
use xd_art_core::par;
use xd_art_core::pool::Pool;
use xd_art_core::settings::{Dither, DitherMode, Metric};

fn bench(c: &mut Criterion) {
    let img = xd_art_bench::photo_like(640, 480, 11).to_work();
    let pal = palette::builtin("pico-8").expect("built-in");
    let pool = Pool::new();
    let cancel = Cancel::new();
    let all = std::thread::available_parallelism().map_or(1, usize::from);
    let mut g = c.benchmark_group("map 640x480 to pico-8, redmean");
    g.sample_size(20);
    for mode in [
        DitherMode::None,
        DitherMode::Bayer4,
        DitherMode::Floyd,
        DitherMode::Stucki,
    ] {
        let d = Dither {
            mode,
            amount: 0.4,
            ..Dither::default()
        };
        for threads in [1, all] {
            let id = BenchmarkId::new(mode.name(), format!("{threads} threads"));
            g.bench_function(id, |b| {
                par::with_threads(threads, || {
                    b.iter(|| {
                        let out =
                            dither::map(&img, pal.colors(), Metric::Redmean, &d, &pool, &cancel)
                                .expect("not cancelled");
                        pool.recycle_indexed(out);
                    });
                });
            });
        }
    }
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
