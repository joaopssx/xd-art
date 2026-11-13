//! Quick speed check against `speed-budget.txt`, for CI.
//!
//! ```sh
//! cargo run --release -p xd-art-bench --example speed_check
//! ```
//!
//! Each case runs a whole conversion on one thread (so the core count of the machine
//! does not matter) with a fresh pipeline (so nothing is cached), 7 times, and the
//! median is compared with the budget. The budgets are about four times what the cases
//! take on the development machine: CI runners are slower and noisy, and the check is
//! there to catch a change that makes something several times slower, not a few
//! percent. Exits with 1 when a case is over its budget.

use std::time::{Duration, Instant};

use xd_art_core::cancel::Cancel;
use xd_art_core::par;
use xd_art_core::pipeline::Pipeline;
use xd_art_core::settings::{DitherMode, SampleMode, Settings, Size};

const RUNS: usize = 7;

fn size(width: u32, height: u32) -> Size {
    Size::Absolute {
        width,
        height,
        keep_aspect: false,
    }
}

/// The cases: name, source size, settings.
fn cases() -> Vec<(&'static str, (u32, u32), Settings)> {
    let base = |w, h, mode, dither| {
        let mut s = Settings::default();
        s.sample.size = size(w, h);
        s.sample.mode = mode;
        s.dither.mode = dither;
        s
    };
    let mut blurred = base(160, 90, SampleMode::Nearest, DitherMode::None);
    blurred.blur.radius = 8.0;
    vec![
        (
            "nearest-160x90",
            (1920, 1080),
            base(160, 90, SampleMode::Nearest, DitherMode::None),
        ),
        (
            "lanczos-960x540",
            (1920, 1080),
            base(960, 540, SampleMode::Lanczos, DitherMode::None),
        ),
        ("blur8-160x90", (1920, 1080), blurred),
        (
            "bayer4-640x480",
            (640, 480),
            base(640, 480, SampleMode::Nearest, DitherMode::Bayer4),
        ),
        (
            "floyd-640x480",
            (640, 480),
            base(640, 480, SampleMode::Nearest, DitherMode::Floyd),
        ),
    ]
}

fn budgets(text: &str) -> Vec<(String, f64)> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let mut p = l.split_whitespace();
            Some((p.next()?.to_string(), p.next()?.parse().ok()?))
        })
        .collect()
}

fn main() {
    let text = include_str!("../speed-budget.txt");
    let budgets = budgets(text);
    let mut over = 0;
    println!("{:<18}{:>12}{:>12}{:>8}", "case", "median", "budget", "");
    for (name, (w, h), settings) in cases() {
        let img = xd_art_bench::photo_like(w, h, 7);
        let mut times: Vec<Duration> = (0..RUNS)
            .map(|_| {
                par::with_threads(1, || {
                    let mut p = Pipeline::default();
                    let t = Instant::now();
                    p.run(&img, 1, &settings, &Cancel::new())
                        .expect("conversion");
                    t.elapsed()
                })
            })
            .collect();
        times.sort();
        let ms = times[RUNS / 2].as_secs_f64() * 1000.0;
        let budget = budgets.iter().find(|(n, _)| n == name).map(|(_, b)| *b);
        let verdict = match budget {
            Some(b) if ms > b => {
                over += 1;
                "OVER"
            }
            Some(_) => "ok",
            None => {
                over += 1;
                "no budget"
            }
        };
        let shown = budget.map_or_else(|| "-".to_string(), |b| format!("{b:.1} ms"));
        println!("{name:<18}{:>9.2} ms{shown:>12}{verdict:>8}", ms);
    }
    if over > 0 {
        eprintln!("{over} case(s) over budget or without one");
        std::process::exit(1);
    }
}
