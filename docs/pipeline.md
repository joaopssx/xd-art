# The pipeline

`xd_art_core::pipeline::Pipeline` runs a conversion as seven stages, always in this
order:

```text
source -> Blur -> Sample -> Adjust ----------> Map -> Post -> Output
                               \-> Palette --/
```

| stage | input | output |
|---|---|---|
| Blur | the source | the source, blurred (or untouched with radius 0) |
| Sample | Blur | the image at the target size |
| Adjust | Sample | sharpened, brightness/contrast/saturation/hue/gamma/tint applied |
| Palette | Adjust (only when extracting) | the palette |
| Map | Adjust, Palette, the mask | an indexed image, dithered or not |
| Post | Map | the indexed image after outline, inline, cleanup, merge |
| Output | Post, Palette | RGBA pixels, scaled up by the export scale |

The order is the original program's. The specification of this project listed the
color adjustments before sampling; the original runs them on the small image, which is
both what its results depend on and much cheaper, so xd-art does the same
(`docs/decisions.md`).

## Running only what changed

Every stage has a key: a hash of its own settings and the keys of the stages it reads.
The source is identified by a number the caller picks. On `run`, a stage whose key did
not change is skipped and its last result reused. So moving the dither amount runs Map,
Post and Output; changing the width runs everything from Sample on. A built-in palette
does not depend on the image, so it is not rebuilt when brightness moves; an extracted
palette is. A test checks the exact set of stages each setting invalidates.

Besides the current result of each stage, the pipeline keeps a few older ones, so
switching an option back and forth is served from memory. `CachePolicy` bounds the
bytes they may hold; the least recently used go first.

`run` returns a `Report`: which stages ran, which came from the older results, and the
time each one took.

## Threads, buffers, cancelling

- Stages split their work by rows with rayon (feature `parallel`, on by default). The
  number of threads never changes the result.
- Buffers come from a `Pool` and go back to it, so converting the next frame of the
  same size allocates nothing for pixels.
- A `Cancel` token is checked between rows; a cancelled run returns
  `Error::Cancelled` within one row of work.

## In the app

The live preview adds three things on top (details and numbers in
`docs/performance.md`):

- Sources over 1.5 megapixels get a copy reduced to 1024 pixels on the long side. A new
  image is converted from the copy first and shown, then from the full source.
- While a slider is dragged, only the reduced copy is converted and a LAB metric is
  replaced by redmean. The exact conversion runs 120 ms after the last change. The
  status bar says "preview" until then; clicking it runs the exact one at once.
- Results are tagged with a generation number. Only the latest generation is shown.

## In GIFs

`anim::convert` runs one pipeline per worker thread over the frames, keeps their order,
and either builds one palette from all frames together (shared, the default) or one per
frame.
