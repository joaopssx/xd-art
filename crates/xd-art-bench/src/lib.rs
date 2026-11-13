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

/// A small landscape for documentation pictures: a sky gradient, a sun with a soft
/// edge, two hills, a few one pixel lines and a checker patch. Each part shows a
/// different weakness: gradients for dithering, the soft edge for blur and sampling,
/// thin lines for nearest sampling, the checker for aliasing.
#[must_use]
pub fn scene(width: u32, height: u32) -> Image {
    let (w, h) = (width.max(1) as f32, height.max(1) as f32);
    let mut img = Image::new(width.max(1), height.max(1), [0, 0, 0]).expect("size is valid");
    let mix = |a: [f32; 3], b: [f32; 3], t: f32| {
        let t = t.clamp(0.0, 1.0);
        [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t)
    };
    for y in 0..height {
        for x in 0..width {
            let (fx, fy) = (x as f32 / w, y as f32 / h);
            // Sky: deep blue at the top to warm orange at the horizon.
            let mut c = mix([24.0, 32.0, 92.0], [250.0, 160.0, 90.0], fy / 0.65);
            // Sun with a soft edge.
            let d = ((fx - 0.68).powi(2) + (fy - 0.38).powi(2)).sqrt();
            c = mix(c, [255.0, 236.0, 150.0], (0.16 - d) / 0.03);
            // Two hills, the near one darker.
            let far = 0.62 + 0.06 * (fx * 7.0).sin();
            if fy > far {
                c = mix([70.0, 110.0, 80.0], [40.0, 70.0, 55.0], (fy - far) / 0.3);
            }
            let near = 0.74 + 0.05 * (fx * 4.0 + 1.0).cos();
            if fy > near {
                c = mix([46.0, 80.0, 40.0], [20.0, 34.0, 22.0], (fy - near) / 0.25);
            }
            // One pixel lines in the sky, like wires.
            if (x as i64 - y as i64 * 2).rem_euclid(97) == 0 && fy < 0.55 {
                c = [20.0, 20.0, 30.0];
            }
            // A checker patch in the bottom left corner.
            if fx < 0.22 && fy > 0.84 && (x / 2 + y / 2) % 2 == 0 {
                c = [230.0, 230.0, 220.0];
            }
            img.set(x, y, [c[0] as u8, c[1] as u8, c[2] as u8, 255]);
        }
    }
    img
}
