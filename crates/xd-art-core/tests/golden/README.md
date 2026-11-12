# Golden images

These files are outputs of the original SLK_img2pixel algorithms, used by
`tests/golden.rs` to check that xd-art computes the same pixels.

They are produced by `scripts/gen-goldens.sh`, which compiles only the algorithm sources
of the original (blur, sampling, sharpen, color adjustments, dithering, k-means) with a
small harness, `scripts/golden-harness.c`, and runs it. The original's GUI needs SDL,
which is why the harness exists at all. `SOURCE` holds the commit of SoftLK-tools the
files were made from.

Inputs are synthetic: `in_gradient` and `in_noise` (48x40, the noise has an alpha ramp in
its 8 leftmost columns). The tests rebuild both in Rust and check them against these
files first, so a mismatch in the generators shows up on its own.

## Format

`XDG1`, then width and height as little endian `u32`, then RGBA bytes. Small, and
readable without any image library on either side.

## What is covered

| prefix | what |
|---|---|
| `sample_*` | the five sampling modes of the original, down to 17x13 and up to 70x50 with offsets |
| `blur_*` | the original blur at radius 0.5, 1.5 and 4.25 |
| `adjust_*` | sharpen, hue/saturation/contrast/brightness, gamma and tint on a 24x20 image |
| `dither_*` | every dither mode with PICO-8 and redmean, plus two cases with transparency |
| `metric_*` | closest color with each of the six distance metrics |

The median cut assignment mode has no golden. The original sorts with `qsort`, which is
not stable, so its result depends on the C library. See `docs/compat.md`.

## Regenerating

```sh
scripts/gen-goldens.sh                    # clones the original into a temp dir
scripts/gen-goldens.sh ~/src/SoftLK-tools # or uses a checkout you already have
```

Two runs on the same commit produce identical files.
