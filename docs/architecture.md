# Architecture

xd-art is a Cargo workspace. The engine knows nothing about windows, terminals or the
web; everything else is a front end that drives it.

```text
                 xd-art-text (strings, en/pt)
                   ^          ^
                   |          |
 xd-art-gui (gpui) |   xd-art-cli ----> xd-art-lua (mlua, vendored Lua 5.4)
        \          |      /                 /
         \         |     /                 /
          +--------+----+-----------------+
                   v
             xd-art-core  <---- xd-art-wasm (browser)
                   ^
                   |
             xd-art-bench (benchmarks, speed check, doc pictures)
```

| crate | what it is |
|---|---|
| `xd-art-core` | the engine: image types, I/O, settings, the pipeline and every algorithm, palettes, presets, recipes, exporters |
| `xd-art-gui` | the desktop app (`xd-art` binary) on gpui |
| `xd-art-cli` | the command line tool (`xd-art-cli`) |
| `xd-art-lua` | the Lua bridge used by `xd-art-cli run` |
| `xd-art-wasm` | the engine for the browser, with `web/` around it |
| `xd-art-text` | interface strings in English and Portuguese |
| `xd-art-bench` | criterion benchmarks, the CI speed check, synthetic test images |

## The engine

`xd-art-core` is organized by stage, with the types they share:

| module | |
|---|---|
| `buffer` | `Image` (packed RGB plus an optional alpha plane), the 15 bit `Work` format the stages compute in, `Indexed` |
| `io` | load and save PNG, JPEG (in), BMP, TGA, QOI; memory limit and reduced decoding for huge files |
| `settings` | every option, with defaults equal to the original's, validation and the fingerprints the cache keys are made of |
| `pipeline` | the stage graph, dirty tracking, the per-stage cache, cancellation |
| `prep` | blur and the color adjustments |
| `sampling` | the seven ways to make the image small |
| `color` | sRGB, linear and Lab, the six distance metrics, the nearest-color cache, k-d tree and lookup table |
| `palette`, `quant` | palettes (files, built-ins, Lospec) and extracting a palette from an image |
| `dither` | ordered and error diffusion dithering, masks |
| `post` | outline, inline, stray pixel cleanup, merging colors |
| `anim` | GIF import, conversion of all frames, GIF export |
| `export` | effects (scanlines, CRT), sprite sheets, indexed PNG, editor handoff |
| `preset`, `recipe` | style presets and recipe files |
| `suggest` | a suggested output size |
| `par`, `pool`, `cancel` | threads, reusable buffers, cancel tokens |

Tables that never change (Bayer and cluster matrices, the sRGB to linear curve) are
computed by `build.rs` and compiled in.

Things the engine guarantees, each covered by tests:

- The same input and settings give the same pixels on any number of threads. Ordered
  dithering splits rows; error diffusion runs as a wavefront that keeps the one thread
  order (`dither.rs`).
- For every option the original program has, the output matches it, or the difference
  is listed in `docs/compat.md` (golden tests in `crates/xd-art-core/tests/golden`).
- A second conversion of the same size does not allocate pixel buffers
  (`tests/alloc.rs`).
- Long work checks a cancel token between rows and stops within one row.

## The desktop app

One window, one root view (`app.rs`). Everything heavy runs on gpui's background
executor; the UI thread only builds elements and draws a CPU upscaled copy of the
visible part of the result (`preview.rs`). A settings change bumps a generation number
and cancels the running conversion; a result whose generation is not the latest is
dropped, so an old image never replaces a new one.

The modules match what is on screen: `panels` (the settings in pipeline order),
`widgets` (the slider), `strip` (palette strip), `gallery`, `history`, `variations`,
`frames` (GIF frames), `retouch`, `exporting`, `commands` (command palette and shortcut
sheet), `theme`, and `config` (the small binary preferences file). `actions.rs` holds
every shortcut in one table. `trace.rs` is the startup trace, `resident.rs` the optional
resident instance.

## The command line tool

`xd-art-cli` parses options with lexopt into the engine's `Settings` (`args.rs`), so
every option has one parser shared by `convert` and `batch`. Exit codes: 0 success,
1 a conversion failed, 2 a usage error.

## Where to look

- How a conversion runs, step by step: `docs/pipeline.md`.
- What each algorithm does, with pictures: `docs/algorithms.md`.
- Why things are the way they are: `docs/decisions.md`.
- Numbers: `docs/performance.md`.
