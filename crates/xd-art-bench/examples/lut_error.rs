//! How often the 3D lookup table disagrees with the exact closest color search, over
//! every one of the 16,777,216 RGB colors, and how long each lookup takes.
//!
//! `cargo run --release -p xd-art-bench --example lut_error`

use std::time::Instant;

use xd_art_core::color::{Cache, Lut, Matcher, coords, distance};
use xd_art_core::palette;
use xd_art_core::settings::Metric;

fn main() {
    for (name, metric) in [
        ("pico-8", Metric::Redmean),
        ("pico-8", Metric::Rgb),
        ("console-32", Metric::Redmean),
        ("web-216", Metric::Redmean),
        ("pico-8", Metric::Cie76),
    ] {
        let pal = palette::builtin(name).expect("built-in");
        let m = Matcher::new(pal.colors(), metric);
        let t = Instant::now();
        let lut = Lut::new(&m);
        let build = t.elapsed();
        let (mut wrong, mut worst) = (0u64, 0f32);
        let t = Instant::now();
        for v in 0..1u32 << 24 {
            let c = [(v >> 16) as u8, (v >> 8) as u8, v as u8];
            let exact = m.closest(c);
            let approx = lut.closest(&m, c);
            if exact != approx {
                wrong += 1;
                let p = coords(metric, c);
                let de = distance(metric, p, coords(metric, pal.colors()[exact as usize]));
                let da = distance(metric, p, coords(metric, pal.colors()[approx as usize]));
                worst = worst.max(da - de);
            }
        }
        let both = t.elapsed();
        println!(
            "{name:>10} {:>9}: lut build {:>6.1} ms, {wrong} of 16777216 colors differ ({:.4}%), worst extra distance {worst:.5}, full sweep {:.1} s",
            metric.name(),
            build.as_secs_f64() * 1000.0,
            wrong as f64 * 100.0 / 16_777_216.0,
            both.as_secs_f64()
        );
    }

    // Lookup cost on a photo-like image, where colors repeat.
    let img = xd_art_bench::photo_like(1024, 1024, 3);
    let px: Vec<[u8; 3]> = img
        .rgb()
        .chunks_exact(3)
        .map(|p| [p[0], p[1], p[2]])
        .collect();
    let pal = palette::builtin("web-216").expect("built-in");
    let m = Matcher::new(pal.colors(), Metric::Redmean);
    let lut = Lut::new(&m);
    let time = |f: &mut dyn FnMut([u8; 3]) -> u8| {
        let t = Instant::now();
        let mut acc = 0u64;
        for &c in &px {
            acc += u64::from(f(c));
        }
        (t.elapsed().as_secs_f64() * 1e9 / px.len() as f64, acc)
    };
    let (exact, a) = time(&mut |c| m.closest(c));
    let mut cache = Cache::default();
    let (cached, b) = time(&mut |c| cache.closest(&m, c));
    let (looked, _) = time(&mut |c| lut.closest(&m, c));
    assert_eq!(a, b);
    println!(
        "web-216 redmean, 1024x1024 photo-like, one thread: exact {exact:.1} ns/px, cache {cached:.1} ns/px ({} hits, {} misses), lut {looked:.1} ns/px",
        cache.hits, cache.misses
    );
}
