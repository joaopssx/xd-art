//! xd-art's engine compiled to WebAssembly for `web/index.html`.
//!
//! Everything runs in the page: the image never leaves the browser. The API is small on
//! purpose: the page builds a recipe (the same JSON the CLI and the desktop app use),
//! hands it over with the image bytes, and gets a PNG back. See `docs/web.md`.

use wasm_bindgen::prelude::*;
use xd_art_core::cancel::Cancel;
use xd_art_core::io::{self, Format, LoadOptions};
use xd_art_core::pipeline::Pipeline;
use xd_art_core::settings::{DitherMode, Metric, SampleMode, Settings};

/// Converts an encoded image (PNG, JPEG, BMP or TGA) with a recipe and returns PNG
/// bytes.
///
/// # Errors
///
/// Fails when the image cannot be decoded, the recipe is invalid, or the conversion
/// fails; the message says which.
#[wasm_bindgen]
pub fn convert(image: &[u8], recipe: &str) -> Result<Vec<u8>, JsError> {
    convert_bytes(image, recipe).map_err(|e| JsError::new(&e))
}

/// The recipe of the default settings, as a starting point for the page.
#[wasm_bindgen]
#[must_use]
pub fn default_recipe() -> String {
    xd_art_core::recipe::to_json(&Settings::default())
}

/// Names for the page's menus: `"palettes"`, `"samplers"`, `"dithers"` or `"metrics"`.
/// Anything else gives an empty list.
#[wasm_bindgen]
#[must_use]
pub fn names(kind: &str) -> Vec<String> {
    let v: Vec<&str> = match kind {
        "palettes" => xd_art_core::palette::builtin_names().collect(),
        "samplers" => SampleMode::ALL.iter().map(|m| m.name()).collect(),
        "dithers" => DitherMode::ALL.iter().map(|m| m.name()).collect(),
        "metrics" => Metric::ALL.iter().map(|m| m.name()).collect(),
        _ => Vec::new(),
    };
    v.into_iter().map(String::from).collect()
}

/// [`convert`] without the JavaScript error type, so it can be tested natively.
fn convert_bytes(image: &[u8], recipe: &str) -> Result<Vec<u8>, String> {
    let img = io::decode_with(image, &LoadOptions::default())
        .map_err(|e| e.to_string())?
        .image;
    let settings = xd_art_core::recipe::from_json(recipe).map_err(|e| e.to_string())?;
    let mut p = Pipeline::default();
    p.run(&img, 1, &settings, &Cancel::new())
        .map_err(|e| e.to_string())?;
    let out = p.output().ok_or("the conversion gave no image")?;
    io::encode(out, Format::Png).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use xd_art_core::buffer::Image;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut img = Image::new(w, h, [0, 0, 0]).unwrap();
        for x in 0..w {
            img.set(x, x % h, [255, 200, 40, 255]);
        }
        io::encode(&img, Format::Png).unwrap()
    }

    #[test]
    fn converts_with_the_default_recipe() {
        let mut s = Settings::default();
        s.sample.size = xd_art_core::settings::Size::Absolute {
            width: 8,
            height: 4,
            keep_aspect: false,
        };
        let out = convert_bytes(&png(32, 16), &xd_art_core::recipe::to_json(&s)).unwrap();
        let img = io::decode_with(&out, &LoadOptions::default())
            .unwrap()
            .image;
        assert_eq!((img.width(), img.height()), (8, 4));
        assert!(convert_bytes(&png(4, 4), &default_recipe()).is_ok());
    }

    #[test]
    fn errors_name_the_problem() {
        assert!(convert_bytes(b"not an image", &default_recipe()).is_err());
        let e = convert_bytes(&png(4, 4), "{").unwrap_err();
        assert!(!e.is_empty());
    }

    #[test]
    fn menus_are_filled() {
        assert!(names("palettes").contains(&"pico-8".to_string()));
        assert_eq!(names("dithers").len(), DitherMode::ALL.len());
        assert!(names("nothing").is_empty());
    }
}
