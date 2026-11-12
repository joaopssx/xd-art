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
