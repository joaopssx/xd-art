//! Closest color search: linear against kd-tree, by palette size.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use xd_art_core::color::Matcher;
use xd_art_core::settings::Metric;

fn colors(n: usize, seed: u32) -> Vec<[u8; 3]> {
    let mut s = seed;
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            [s as u8, (s >> 8) as u8, (s >> 16) as u8]
        })
        .collect()
}

fn palette(c: &mut Criterion) {
    let queries = colors(4096, 99);
    for metric in [Metric::Redmean, Metric::Rgb] {
        let mut g = c.benchmark_group(format!("closest of 4096, {}", metric.name()));
        for n in [16, 24, 32, 48, 64, 128, 256] {
            let m = Matcher::new(&colors(n, 7), metric);
            g.bench_with_input(BenchmarkId::new("linear", n), &n, |b, _| {
                b.iter(|| {
                    queries
                        .iter()
                        .map(|&q| u32::from(m.closest_linear(q)))
                        .sum::<u32>()
                });
            });
            if m.has_tree() {
                g.bench_with_input(BenchmarkId::new("kd-tree", n), &n, |b, _| {
                    b.iter(|| {
                        queries
                            .iter()
                            .map(|&q| u32::from(m.closest(q)))
                            .sum::<u32>()
                    });
                });
            }
        }
        g.finish();
    }
}

criterion_group!(benches, palette);
criterion_main!(benches);
