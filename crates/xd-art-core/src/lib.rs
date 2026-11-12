//! The xd-art engine: turns images into pixel art.
//!
//! The crate has no user interface code. The GUI, the command line tool, the Lua
//! bridge and the web build all drive the same [`pipeline`].
//!
//! Stages run in a fixed order: blur the source, sample it down to the target
//! size, adjust colors on the small image, pick or build a palette, map pixels to
//! the palette (with or without dithering), then post-process.

#![warn(clippy::pedantic)]

pub mod anim;
pub mod buffer;
pub mod cancel;
pub mod color;
pub mod dither;
pub mod error;
pub mod export;
pub mod io;
pub mod palette;
pub mod pipeline;
pub mod pool;
pub mod post;
pub mod prep;
pub mod preset;
pub mod quant;
pub mod recipe;
pub mod sampling;
pub mod settings;
pub mod suggest;

pub use error::{Error, Result};
