//! Writes the synthetic test image the benchmarks use, for scripts that need it as a
//! file.
//!
//! ```sh
//! cargo run --release -p xd-art-bench --example make_image -- out.png 1920 1080
//! ```

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [path, w, h] = args.as_slice() else {
        eprintln!("usage: make_image <out.png> <width> <height>");
        std::process::exit(2);
    };
    let (Ok(w), Ok(h)) = (w.parse(), h.parse()) else {
        eprintln!("width and height must be numbers");
        std::process::exit(2);
    };
    let img = xd_art_bench::photo_like(w, h, 7);
    if let Err(e) = xd_art_core::io::save(&img, std::path::Path::new(path)) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
