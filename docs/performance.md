# Performance

Numbers here are measured, with the command next to them. Unless a section says
otherwise, the machine is Windows 11 Pro, AMD Ryzen 9 7950X (16 cores, 32 threads),
32 GB RAM, rustc 1.95.0, release profile from the workspace `Cargo.toml`.

Criterion reports a range; the tables show its middle value.

## Sampling

```sh
cargo bench -p xd-art-bench --bench resize -- --warm-up-time 1 --measurement-time 2 sample
```

Input is a synthetic 1920x1080 image (`xd_art_bench::photo_like`).

| mode | 160x90, first port | 160x90, now | 960x540, first port | 960x540, now |
|---|---|---|---|---|
| nearest | 29.2 µs | 29.7 µs | | |
| bilinear | 162 µs | 145 µs | 5.76 ms | 0.78 ms |
| bicubic | 1.04 ms | 0.25 ms | | |
| lanczos | 1.92 ms | 0.19 ms | 68.0 ms | 1.66 ms |
| box | 1.10 ms | 0.20 ms | | |
| cluster | 69.3 ms | 4.86 ms | | |

"First port" is the straight translation of the original's loops, one thread. "Now" is
the same math with three changes:

- Rows run in parallel with rayon, in groups sized by how much work a pixel is (a
  Lanczos sample reads 36 source pixels, a nearest sample one), so a 160x90 nearest
  resize stays on one thread and a 160x90 cluster resize uses all of them.
- Values that only depend on the column (source position, fraction, the six Lanczos
  weights with their `sin` calls) are computed once per image instead of once per
  pixel. That alone is most of the Lanczos gain.
- The four channels go through the filters as `[f32; 4]`, with the same operations in
  the same order as the scalar code. LLVM turns those into SSE instructions. Each
  channel still sees the exact sequence of `f32` roundings it had before, so the golden
  tests did not move.

There is no hand written SIMD and no `unsafe` in the sampling code. I tried to find a
place where intrinsics would pay off and the remaining time is in memory access (the
source of a 1920x1080 image in the working format is 16 MB), not arithmetic.

### fast_image_resize

```sh
cargo run --release -p xd-art-bench --example fir_diff
```

| filter | max diff 160x90 | channels off by more than 1 | max diff 960x540 | channels off by more than 1 |
|---|---|---|---|---|
| bilinear | 16 | 82.4% | 20 | 75.7% |
| bicubic | 20 | 85.9% | 23 | 80.8% |
| lanczos | 28 | 87.2% | 26 | 83.4% |
| box | 1 | 0.0% | 1 | 0.0% |

Time for the same 160x90 reduction (same bench command): bilinear 247 µs, bicubic
452 µs, lanczos 864 µs, box 134 µs.

The difference is expected: `fast_image_resize` does proper convolution (every source
pixel under the filter contributes, weights normalized, edges clamped). The original
samples the filter at one point per output pixel, with `(size - 1) / size` steps and
black outside the image. Those are two different results, not two speeds of the same
result. See the decision in [decisions.md](decisions.md).

## Blur

```sh
cargo bench -p xd-art-bench --bench blur -- --warm-up-time 1 --measurement-time 3
```

1920x1080 synthetic image, all cores.

| radius | exact gaussian | running box (3 passes) | original box (compatible) |
|---|---|---|---|
| 2 | 6.92 ms | 11.3 ms | 11.5 ms |
| 8 | 18.9 ms | 11.8 ms | 11.0 ms |
| 32 | 66.3 ms | 11.6 ms | 10.9 ms |

The gaussian grows with the kernel; both box versions do not. They cross between radius
4 and 5, so the default `box_from` is 5: from there on the gaussian mode uses the running
box passes.

Accuracy of that switch, from the test `large_radius_approximation_is_close` on a
160x120 noise image (noise is the worst case): the running box result is identical to
the exact gaussian in 8-bit terms away from the edges, and at most 1 step off within
`3 sigma` of an edge, at radius 8 and 20. The first version clamped at the end of each
of the three passes and was up to 21 steps off in the corners; extending the line once
with its end pixels before the passes fixed that.

## Closest color lookups

```sh
cargo run --release -p xd-art-bench --example lut_error
```

The 3D table (32 cells per channel, up to 8 candidate colors per cell, exact distance
among the candidates) against the exact linear search, over all 16,777,216 RGB colors:

| palette | metric | table build | colors that differ | worst extra distance |
|---|---|---|---|---|
| pico-8 | redmean | 10.9 ms | 788 (0.0047%) | 0.019 |
| pico-8 | rgb | 11.1 ms | 447 (0.0027%) | 0.0065 |
| console-32 | redmean | 20.4 ms | 624 (0.0037%) | 0.016 |
| web-216 | redmean | 134 ms | 0 | 0 |
| pico-8 | cie76 | 21.2 ms | 574 (0.0034%) | 90.9 |

"Extra distance" is how much farther the color picked by the table is than the right
one, in the metric's own units (RGB metrics on `0..1` channels, squared). For CIE76 the
unit is squared LAB distance, so 90.9 is about 9.5 delta E: the table cells are cubes in
RGB, which are not where LAB metrics draw their borders. That is fine for the preview
while dragging, which uses a cheap RGB metric anyway, and never used for export.

Per lookup, one thread, web-216 with redmean on a 1024x1024 synthetic photo:

| method | time per pixel |
|---|---|
| exact linear search | 594 ns |
| exact search behind the 4096 slot cache (25% hits on this noisy image) | 361 ns |
| table | 5.9 ns |

The cache helps in proportion to how often colors repeat. On a noisy photo that is one
lookup in four; on an image that was already reduced and smoothed it is most of them.

### kd-tree threshold

```sh
cargo bench -p xd-art-bench --bench palette -- --warm-up-time 1 --measurement-time 2
```

Time for 4096 exact lookups against random palettes, in µs, one thread:

| colors | 16 | 24 | 32 | 48 | 64 | 128 | 256 |
|---|---|---|---|---|---|---|---|
| redmean, linear | 206 | 348 | 404 | 642 | 795 | 1595 | 3137 |
| redmean, kd-tree | 384 | 476 | 527 | 639 | 678 | 838 | 1042 |
| rgb, linear | 180 | 286 | 345 | 515 | 702 | 1636 | 3206 |
| rgb, kd-tree | 353 | 418 | 501 | 556 | 574 | 703 | 825 |

The two meet around 48 colors, so the tree is built from 64 colors up (`KD_FROM`). It
only exists for RGB, weighted, redmean and CIE76, the metrics with a per channel lower
bound; CIE94 and CIEDE2000 always search linearly. The tree gives exactly the linear
search's answer, ties included, which a test checks on 5000 colors for several palette
sizes.

## Dithering

```sh
cargo bench -p xd-art-bench --bench dither -- --warm-up-time 1 --measurement-time 3
```

640x480 synthetic image mapped to PICO-8 with redmean.

| mode | 1 thread | 32 threads |
|---|---|---|
| none (closest color) | 35.2 ms | 2.38 ms |
| bayer 4x4 | 36.4 ms | 2.44 ms |
| Floyd-Steinberg | 45.0 ms | 5.87 ms |
| Stucki | 54.3 ms | 6.78 ms |

Ordered modes split rows across threads. Error diffusion runs as a wavefront: each row
trails the one above by twice the kernel's reach, which keeps the order in which every
pixel receives its error the same as in the one thread loop. The result is identical
(a test compares 1, 3 and 8 threads for every mode on a 320x240 image), and the speedup
is lower than for the ordered modes because rows wait on each other.

## Allocations

`crates/xd-art-core/tests/alloc.rs` counts heap allocations with a replacement global
allocator. A pipeline that already converted one frame converts the next one of the same
size (96x64 to 48x32, Floyd-Steinberg) with 12 allocations, none of them for pixels:
one `Arc` per stage result, the palette, the color matcher and its cache. GIF
conversion keeps one pipeline per thread for exactly this reason.

## Startup

What `xd-art` does from launch to the first frame, in order:

1. Collects the command line arguments as paths. Nothing is opened yet.
2. Starts gpui and creates the window (1280x800, centered).
3. Creates the root view with no image.
4. If a path was given (also the case when a file is dropped on the executable), starts
   decoding it on a background thread. The UI thread never waits for it.
5. Draws the first frame: the empty state, or "opening <file>".

Not done at startup, on purpose: scanning folders for recent files or user palettes,
building any palette, reading or writing anything else than the config file (added
later, one small read), checking for updates, any network access, creating a Lua state.
Each of those happens the first time it is needed, or never.

Timing of these steps is measured in the startup trace section below, once the trace
exists.

## Live preview

Every settings change starts a conversion on a background thread. A newer change
cancels the running one (it stops within a row of work) and results of anything but the
latest request are dropped, so an older image never replaces a newer one. The pipeline
cache means only the stages after the change run.

The status bar shows the time from the request to the result being on screen. With
the 1600x1200 synthetic test image converted to 64x64 with PICO-8 and nearest sampling,
it read 5 ms right after opening (Windows 11, Ryzen 9 7950X, `fast` profile).

Sources above 1.5 megapixels also get a copy reduced to 1024 pixels on the long side.
The first conversion of a new image runs on that copy first and shows it, then runs on
the full image; while a slider is dragged, only the copy is converted.

### While dragging

A slider being dragged converts the reduced copy (for big sources) and swaps a LAB
metric for redmean, the cheapest of the RGB ones; CIEDE2000 in particular costs several
times more per comparison. The result is marked "preview" in the status bar, and a click
there runs the exact conversion at once. Otherwise the exact one starts 120 ms after the
last change, so typing a value or clicking through options runs it once.

### Visible region

The preview draws only what is visible: the image is upscaled on the CPU for exactly the
part of it inside the view, at device resolution. Zooming into a corner of a 512x512
result costs the same as looking at a 64x64 one.

The conversion itself always runs on the whole image. I looked at converting only the
visible part and decided against it. The result is small (64 to 512 pixels on a side in
normal use) and the stages that work on it take a few milliseconds in total; the only
expensive stage is the blur on the full source, which has to see the whole source
anyway because sampling reads all of it. Error diffusion also cannot be cut into
regions without changing the result, since every pixel depends on everything before it.
