# Compatibility with SLK_img2pixel

xd-art started as a port, so for every option the original has, the goal is the same
pixels. This page lists how close that is, checked by the golden tests in
`crates/xd-art-core/tests/golden.rs`, and every place where xd-art does something else
on purpose.

The goldens come from the original's own algorithm sources compiled with a small
harness (`scripts/gen-goldens.sh`); the original's interface needs SDL and was not part
of that build. Commit used: see `crates/xd-art-core/tests/golden/SOURCE`.

## Same output

| stage | result |
|---|---|
| nearest sampling | identical |
| cluster sampling | identical |
| bilinear, bicubic, Lanczos sampling | within 1 step of 255 per channel (tolerance of the test), on the test images identical except a few alpha values |
| blur (compatible mode) | identical |
| sharpen, tint | identical |
| hue/saturation/contrast/brightness, gamma | within 1 step |
| closest color, all six metrics | identical |

## Kept on purpose, although they look like bugs

- **Sampling step.** Output pixel `x` samples source position `(x + offset + 0.5) *
  (source - 1) / target`. The `- 1` shifts the image a little toward the top left.
- **Black outside the image.** Bilinear, bicubic and Lanczos read opaque black for
  pixels past the edge instead of repeating the edge, so the last row and column get
  darker.
- **Lanczos is not normalized.** The six weights do not add up to 1 in general; flat
  areas come out up to about 1% darker.
- **Opaque alpha.** The working format stores an opaque pixel as `255 << 7` but treats
  pixels outside the image as `0x7fff`. Near the edges the two mix.
- **Blur passes.** The original's blur runs three box passes on rows, and on columns it
  runs three as well but copies back the buffer that holds the second pass. Columns get
  two passes. xd-art's compatible blur does the same.
- **CIEDE2000 rotation term.** The last term of the formula multiplies by the mean
  hue `H'` where the standard has the hue difference `ΔH'`. It changes which palette
  color wins for some inputs (146 of 3072 channels in the metric golden test before I
  copied it). The other parts of the formula are standard, including the `f32`
  constants promoted to `f64`.

Changing any of these would make xd-art's results differ from the original's for the
same settings, which is the one thing the port promises not to do.

## Different on purpose

- **Short lines in the blur.** For lines shorter than `2r + 2` pixels the original's box
  filter reads and writes past the end of its buffers (undefined behavior in C).
  xd-art clamps reads to the last pixel and drops the extra writes. The original skips
  images not larger than the radius, so this only matters for radii close to the image
  size.
- **Empty k-means clusters.** The original refills an empty cluster with a pixel picked
  by C's `rand()`, which depends on the C library and, with OpenMP, on thread timing.
  xd-art picks it with its own seeded generator. On the test images the case never
  happens and the results are identical.
- **Default blur.** xd-art's default blur is a true gaussian with the same overall
  strength for the same radius (sigma is `sqrt(r * (r + 1))`). The original's three box
  blur is available as the compatible mode (`box_approx` in the settings).
- **Default palette.** PICO-8 instead of DawnBringer 32, see
  [decisions.md](decisions.md).
