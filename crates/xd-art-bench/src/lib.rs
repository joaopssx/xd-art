//! Shared inputs for the xd-art benchmarks.
//!
//! Every benchmark runs on synthetic images so the numbers do not depend on files that
//! are not in the repository.

use xd_art_core::buffer::Image;

/// A noisy image with smooth gradients underneath, so filters and palettes have real
/// work to do. Same seed, same pixels.
#[must_use]
pub fn photo_like(width: u32, height: u32, seed: u32) -> Image {
    let mut s = seed.max(1);
    let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
    for y in 0..height {
        for x in 0..width {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            let n = (s & 31) as i32 - 16;
            let base = [
                x * 255 / width.max(1),
                y * 255 / height.max(1),
                (x + y) * 128 / (width + height).max(1),
            ];
            for b in base {
                rgb.push((b as i32 + n).clamp(0, 255) as u8);
            }
        }
    }
    Image::from_parts(width, height, rgb, None).expect("size is valid")
}
