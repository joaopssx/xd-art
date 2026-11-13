//! The xd-art engine: turns images into pixel art.
//!
//! The crate has no user interface code. The GUI, the command line tool, the Lua
//! bridge and the web build all drive the same [`pipeline`].
//!
//! Stages run in a fixed order: blur the source, sample it down to the target
//! size, adjust colors on the small image, pick or build a palette, map pixels to
//! the palette (with or without dithering), then post-process.
//!
//! ```
//! use xd_art_core::buffer::Image;
//! use xd_art_core::cancel::Cancel;
//! use xd_art_core::pipeline::Pipeline;
//! use xd_art_core::settings::{DitherMode, Settings, Size};
//!
//! // Any image works; this one is a flat orange square with a white stripe.
//! let mut img = Image::new(128, 128, [230, 120, 40])?;
//! for x in 0..128 {
//!     img.set(x, 64, [255, 255, 255, 255]);
//! }
//!
//! let mut s = Settings::default();
//! s.sample.size = Size::Absolute { width: 32, height: 32, keep_aspect: false };
//! s.dither.mode = DitherMode::Floyd;
//!
//! let mut p = Pipeline::default();
//! p.run(&img, 1, &s, &Cancel::new())?;
//! let out = p.output().expect("a finished run has an output");
//! assert_eq!((out.width(), out.height()), (32, 32));
//! # Ok::<(), xd_art_core::Error>(())
//! ```
//!
//! [`io::load`] and [`io::save`] read and write files, [`recipe`] stores settings as
//! JSON, and [`palette::builtin`] gives the palettes that come with the crate.

#![warn(clippy::pedantic)]
// Pixel code converts between u8, u16, i32 and f32 all the time, always on values whose
// range is known (channels, coordinates of images that already fit in memory). Spelling
// each one with try_from would hide the math.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless
)]
// x, y, r, g, b, a are the natural names in image code.
#![allow(clippy::many_single_char_names, clippy::similar_names)]
// Channel loops index two or three arrays with the same `c`; iterators make that worse.
#![allow(clippy::needless_range_loop)]
// Ported code compares floats exactly where the original does (hue sectors, "is this
// setting at its default"); a tolerance would change results.
#![allow(clippy::float_cmp)]
// Counts here run over a handful of neighbours, not buffers worth a dependency.
#![allow(clippy::naive_bytecount)]

#[cfg(feature = "gif")]
pub mod anim;
pub mod buffer;
pub mod cancel;
pub mod color;
pub mod dither;
pub mod error;
pub mod export;
pub mod io;
pub mod palette;
pub mod par;
pub mod pipeline;
pub mod pool;
pub mod post;
pub mod prep;
pub mod preset;
pub mod quant;
#[cfg(feature = "serde")]
pub mod recipe;
pub mod sampling;
pub mod settings;
pub mod suggest;

/// Tables computed by `build.rs`.
#[allow(clippy::unreadable_literal, clippy::excessive_precision)]
mod tables {
    include!(concat!(env!("OUT_DIR"), "/tables.rs"));
}

pub use error::{Error, Result};
