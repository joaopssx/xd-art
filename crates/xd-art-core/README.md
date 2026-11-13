# xd-art-core

The engine behind [xd-art](https://github.com/joaopssx/xd-art). It takes an image and
turns it into pixel art: blur, sample down, adjust colors, map to a palette with or
without dithering, then post-process.

It has no user interface code and can be used on its own.

```rust
use xd_art_core::{cancel::Cancel, pipeline::Pipeline, settings::Settings};

fn main() -> xd_art_core::Result<()> {
    let image = xd_art_core::io::load("photo.jpg".as_ref())?;
    let mut pipeline = Pipeline::default();
    pipeline.run(&image, 1, &Settings::default(), &Cancel::new())?;
    xd_art_core::io::save(pipeline.output().unwrap(), "pixel.png".as_ref())?;
    Ok(())
}
```

`examples/convert.rs` is the same with a recipe file. Settings, recipes, palettes,
presets and the exporters are documented on docs.rs; the main repository has the
command line tool, the desktop interface and longer guides in `docs/`.

## Features

| feature | default | what it does |
|---|---|---|
| `parallel` | yes | splits work across threads with rayon; output is identical either way |
| `gif` | yes | animated GIF import and export |
| `qoi` | yes | QOI image format |
| `serde` | yes | recipe files and JSON output |
| `online-palettes` | no | download palettes from Lospec |

License: MIT.
