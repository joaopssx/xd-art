//! The smallest complete use of the engine: load, convert, save.
//!
//! ```sh
//! cargo run --release -p xd-art-core --example convert -- photo.jpg out.png [recipe.json]
//! ```
//!
//! Without a recipe the default settings are used (64x64, nearest sampling, PICO-8,
//! no dithering). `examples/recipes/` in the repository has a few recipes to try.

use std::path::Path;

use xd_art_core::cancel::Cancel;
use xd_art_core::pipeline::Pipeline;
use xd_art_core::settings::Settings;

fn main() -> xd_art_core::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (input, output) = match args.as_slice() {
        [i, o, ..] => (i, o),
        _ => {
            eprintln!("usage: convert <input> <output.png> [recipe.json]");
            std::process::exit(2);
        }
    };
    let settings = match args.get(2) {
        Some(path) => {
            let text = std::fs::read_to_string(path).map_err(|source| xd_art_core::Error::Io {
                path: path.into(),
                source,
            })?;
            xd_art_core::recipe::from_json(&text)?
        }
        None => Settings::default(),
    };

    let image = xd_art_core::io::load(Path::new(input))?;
    let mut pipeline = Pipeline::default();
    let report = pipeline.run(&image, 1, &settings, &Cancel::new())?;
    let out = pipeline.output().expect("a finished run has an output");
    xd_art_core::io::save(out, Path::new(output))?;

    let ms: f64 = report.times.iter().map(|t| t.as_secs_f64() * 1000.0).sum();
    println!(
        "{input} -> {output}: {} x {}, {ms:.1} ms",
        out.width(),
        out.height()
    );
    Ok(())
}
