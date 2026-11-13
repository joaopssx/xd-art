# Decisions

Short notes on choices that are not obvious from the code. Newest at the bottom. Dates are
when the decision was made.

## 2026-11-12: edition 2024 and resolver 3

gpui is an edition 2024 crate, so the workspace uses the same edition. Resolver 3 is the
default for edition 2024 and also respects `rust-version` when picking dependency
versions. The toolchain is pinned to 1.95.0 in `rust-toolchain.toml`.

## 2026-11-12: release profile

`opt-level = 3`, fat LTO, one codegen unit, `strip = true` and `panic = "abort"` for
release builds. Measured on a gpui hello world, which is most of what the GUI binary
weighs at this point (Windows 11, AMD Ryzen 9 7950X, rustc 1.95.0, gpui 0.2.2):

| profile | exe size |
|---|---|
| cargo default release (dynamic CRT) | 10,743,296 bytes |
| fat LTO, 1 CGU, strip, unwind, static CRT | 6,189,568 bytes |
| same with `panic = "abort"` | 4,951,552 bytes |

The fat LTO build takes about 2.5 minutes from scratch on that machine, against about
1.5 minutes for the default release profile.

There is also a `fast` profile (thin LTO, 16 codegen units, incremental) for working on
the interface, and dev builds compile all dependencies with `opt-level = 2` so decoding and
resizing a large image in a debug build is not painfully slow.

`panic = "abort"` is fine for xd-art because nothing in the code relies on catching
panics. The engine returns errors instead of panicking, mlua does not need unwinding to
report Lua errors (those go through Lua's own error mechanism and come back as `Result`),
and gpui does not catch panics either. With abort, a panic still prints its message, the
process just ends right there.

## 2026-11-12: static CRT on Windows

`.cargo/config.toml` adds `+crt-static` for `x86_64-pc-windows-msvc`. The executables then
run on a clean Windows install without the Visual C++ redistributable. On Linux this does
not apply: gpui loads Wayland, X11, xkbcommon and Vulkan libraries from the system, so a
fully static Linux binary is not possible. See [building.md](building.md).

## 2026-11-12: workspace lints

All crates inherit `unsafe_op_in_unsafe_fn = "deny"`, `missing_docs = "warn"` and
`clippy::undocumented_unsafe_blocks = "warn"`. CI runs clippy with `-D warnings`, so in
practice these are errors. xd-art-core turns on `clippy::pedantic` in its `lib.rs`;
individual pedantic lints are only allowed there with a comment saying why.

## 2026-11-12: gpui 0.2.2 from crates.io

gpui is published on crates.io, so I depend on `gpui = "=0.2.2"` instead of a git
dependency on the Zed repository. A git dependency would pull the whole Zed workspace
into the lock file and tie every build to a specific Zed commit. The exact version pin
(`=`) is on purpose: gpui changes its API between releases and an automatic minor
update could break the build. Upgrading is a deliberate change.

I checked the API against the crate source and its own examples, not from memory. Two
things learned that shape the GUI:

- The image sampler in the Windows (DirectX) and Linux (Blade/Vulkan) renderers is
  hardcoded to linear filtering. There is no option for nearest neighbor, so crisp pixels
  have to come from upscaling on the CPU. See the preview notes in [gui.md](gui.md).
- gpui depends on `image` with its default features, which includes every format the
  crate supports (AVIF, EXR, TIFF, WebP and so on). That weight lands in the GUI binary no
  matter what xd-art-core asks for. The engine itself keeps `default-features = false`.

## 2026-11-12: hand written error type

xd-art-core has one `Error` enum with a manual `Display` impl instead of `thiserror`.
It is about 40 lines and saves a proc-macro dependency (and its compile time) for
every crate that uses the engine.

## 2026-11-12: libc pinned to 0.2.189

Found while checking the Linux build: gpui's HTTP client pulls `xattr 0.2.3`, which no
longer compiles against libc 0.2.190 and later on Linux. The lock file keeps libc at
0.2.189 until gpui moves past that `xattr` version.

## 2026-11-12: image crate with a short format list

Decoding uses `image` 0.25 with `default-features = false` and only `png`, `jpeg`, `bmp`
and `tga` (plus `qoi` behind the feature of the same name). The default feature set
would also pull AVIF, EXR, TIFF, WebP, HDR and rayon into the engine, which the CLI and
the web build do not need. GIF goes through the `gif` crate directly, because animation
needs frame level control that `image` hides.

JPEG is input only. A pixel art result saved as JPEG would smear exactly the hard edges
the program is about.

None of these decoders can decode at a reduced scale, so the large image path decodes
once and then averages integer blocks. A memory limit (1 GiB by default, passed to
`image` as `max_alloc`) rejects files whose header asks for more.

## 2026-11-12: settings split per stage, defaults from the original

`Settings` is one struct per pipeline stage so each stage can be fingerprinted on its
own. The defaults are the ones of the original program's interface (64x64 nearest, no
dithering, redmean, alpha threshold 128, dither amount 0.2).

One default differs: the original starts with the DawnBringer 32 palette. I could not
find a license statement for it from its author, so xd-art starts with PICO-8, whose 16
colors are a published spec of the console. More on palette sources in
[credits.md](credits.md).

Fingerprints use FNV-1a instead of `std`'s hasher, because they must be the same on every
run and every platform (they end up in recipes and in the CI speed check).

## 2026-11-12: stage order follows the original

My first plan had all preparation (blur, sharpen, color adjustments, tint) before
sampling. The original does something smarter: only the blur runs on the full size
source, because its job is to remove detail before sampling. Sharpen and the color
adjustments run on the already small image, where they cost almost nothing. xd-art keeps
that order, which is also what makes its results comparable with the original's.

The pipeline caches the current result of every stage plus up to three older ones per
stage (256 MB total, nothing above 64 MB per result). The older ones exist for the
common "flip between two options" comparison. Large intermediates, mostly a blurred full
size photo, are not kept because recomputing them is cheaper than holding several.

## 2026-11-12: buffer pool and cancellation

Stages take their output buffers from a pool owned by the pipeline, and results the
pipeline drops go back to it. The pool keeps at most eight buffers of each kind. The
allocation test in `crates/xd-art-core/tests/alloc.rs` replaces the global allocator
with a counting one (the only `unsafe` outside the engine proper, and test only).

Cancellation is a shared atomic flag that stages check between rows. A stage result is
stored only after the stage returns, so a cancelled run cannot leave a half written
image behind; the previous result stays until a run completes.

## 2026-11-12: opaque alpha is 255 << 7, not 0x7fff

Found while matching Lanczos against the original: my first buffer code turned an
opaque pixel into alpha `0x7fff` in the working format. The original converts every
channel the same way, so opaque is `255 << 7 = 0x7f80`, while pixels outside the image
in its filters are `0x7fff`. The difference only shows near edges after interpolation,
where the two values mix, and it was enough to move alpha by two steps there. Fixed in the
Lanczos commit, since that is where the golden test caught it.

Lanczos weights are also not normalized in the original. A flat area comes out about 1%
darker. xd-art keeps that, because changing it would break every comparison with the
original; see [compat.md](compat.md).

## 2026-11-12: no fast_image_resize in the engine

I measured `fast_image_resize` 6.1 against the ported filters (numbers in
[performance.md](performance.md)). Its bilinear, Catmull-Rom and Lanczos results differ
from the original's by up to 28 levels per channel on more than 75% of channels, because
it convolves over the whole footprint while the original point samples. Accepting that
would break compatibility, which is the point of those modes. Its box filter is within
one level of xd-art's own, but after parallelizing, xd-art's box is in the same range
(203 µs against 134 µs for a 1920x1080 to 160x90 reduction), not worth a dependency.

The crate stays as a dev-dependency of `xd-art-bench`, only to keep that comparison
reproducible.

## 2026-11-12: rayon for rows, no explicit SIMD

`rayon` (optional, `parallel` feature, on by default) runs independent rows on all cores.
Results do not depend on the thread count: every row is computed by the same code, and a
test compares one thread against eight for every sampling mode.

I did not add intrinsics. Writing the channel math on `[f32; 4]` gets vector instructions
from the compiler with no `unsafe`, and keeps the exact `f32` rounding of the scalar code.
So there is no `unsafe` module to run under Miri yet.

## 2026-11-12: error diffusion wavefront on plain threads

The wavefront workers wait for the row above them with a short spin and then
`yield_now`. That is fine on threads of their own but would block rayon's workers,
which other stages expect to be free, so the wavefront uses `std::thread::scope` with
as many threads as rayon has. Pixels are `AtomicU64` with relaxed loads and stores and
the per row progress counters use release and acquire, which is enough because the lag
guarantees no two threads touch the same pixel at once. No `unsafe` needed.

Images below 65,536 pixels skip the wavefront; starting threads costs more there.

## 2026-11-12: a broken commit and a fix commit

The commit that added the export effects (scanlines, CRT, palette cycling) went in with
`export.rs` not compiling: an `as u32` cast without parentheses before `.min()`. My
check script piped its output through `tail`, which hid the failing exit code. The
next commit fixes the file, and the script now keeps the exit code.

The history keeps both commits instead of rewriting the first one. To stay at the
planned number of steps, the tagged release workflow goes into the same commit as the
release documentation instead of a commit of its own.

## 2026-11-12: platform UI font instead of an embedded one

The plan was one embedded open font family with at most two weights. I did not add it:
the only sources I allow myself to download from for this project are crates, the
original's repository and gpui's documentation, and a font is none of those. xd-art uses
the platform's interface font instead (Segoe UI on Windows, the system font on macOS,
Noto Sans on Linux), through gpui's normal font lookup. Numbers in fields ask for
tabular figures (`tnum`), which Segoe UI supports, so values do not shift sideways as
they change. Embedding a font later only means changing `theme::ui_font`.

## 2026-11-12: theme

Every color is in `crates/xd-art-gui/src/theme.rs`. There are four dark variants and no
light one. A test checks that text, muted text, the accent and the error color reach
WCAG AA contrast (4.5:1) on every surface of every variant. The interface uses no blur,
no large shadows and no gradients: flat surfaces are cheaper to draw and easier on the
eyes next to pixel art.

The icons are drawn by hand as 16x16 SVGs with a 1.5 pixel stroke and compiled into the
binary; gpui parses one the first time it is shown.

The GUI crate is a library with a thin binary on top, so its parts can have unit tests
and the theme can grow ahead of the views that use it.

## 2026-11-13: retouches and the project files

There is no project file format. The retouch layer and the dither mask are saved as two
PNG files next to the source image (`cat.png.retouch.png`, `cat.png.mask.png`) when the
user clicks "save with image", and opening `cat.png` loads them back. Any image editor
can open or fix them, and nothing is written next to the user's files without asking.

The layer is stored in art pixels, before the export scale, and only drawn when the
art size matches the one it was painted at. Changing the size hides it instead of
dropping it. The eraser gives pixels back to the conversion rather than painting them
transparent. Pencil strokes have their own undo list, used while the pencil or the
eraser is active; mask strokes are settings changes and go through the normal history.
The tools are off for animated GIFs, where a single layer over every frame would be
wrong more often than right.

## 2026-11-13: no GPU path (P16)

I measured a wgpu prototype before writing any GPU code into xd-art (numbers in
`docs/performance.md`, probe in `scripts/gpu-probe`). The resize is slower on the GPU
once the image has to go there and back, because the CPU version is already under a
millisecond. Ordered dithering is about 2 ms faster per 640x480 frame. Against that:

- Setting up the GPU takes 330 ms or more, longer than the whole startup. It would have
  to happen lazily on a background thread, and the first conversions would run on the
  CPU anyway.
- The shader works in `f32` while the engine matches the original's integer math
  pixel for pixel. A shader that gives the same bytes is more work than the 2 ms it
  saves, and a second, slightly different result is worse than a slower identical one.
- wgpu is a large dependency for a feature that would be off by default.

A full conversion in the live preview measured 5 ms on the CPU (see "Live preview" in
`docs/performance.md`), so there is little left to win. If larger outputs ever make
ordered dithering the bottleneck, the probe is the starting point.

## 2026-11-13: resident instance over local TCP (S11)

Resident mode is off unless `--resident` is given or the setting is on. It uses a TCP
port on 127.0.0.1 and a random token written to `resident.txt` next to the config file,
instead of a Unix socket on one system and a named pipe on another: one code path, only
the standard library, and the handoff measured 1.5 ms. Other local users can reach the
port but cannot read the token (the file is under `%APPDATA%` on Windows and mode 0600
elsewhere), and a message with the wrong token gets no answer.

gpui on Windows ends the process when its last window closes, so the resident instance
cannot close its window and wait in the background. Closing the window minimizes it and
drops the image and the caches instead; Ctrl+Q quits. A `resident.txt` left behind by a
crash only costs the next launch a failed connection before it starts its own instance.
The config file is now read before gpui starts, since resident mode has to decide
whether to start gpui at all; it is a read of a few hundred bytes (0.09 ms).

## 2026-11-13: web build (E14)

The browser build is a separate crate with three functions and a page without a
framework or bundler: it has to keep working without anyone maintaining a JavaScript
toolchain. It converts on the main thread; the page waits 60 ms after the last change
before converting, and a 64x64 result from a 1920x1080 photo took 39 ms in Node. A Web
Worker would keep the page responsive for big outputs and is the next step if that
becomes a problem.

Running the module for the first time found a real bug: `Instant::now` panics on
`wasm32-unknown-unknown`. The pipeline now leaves stage times at zero there.

## 2026-11-13: engine crate ready to publish, dependency audit (E11, S9)

`xd-art-core` has the metadata crates.io needs, a README, an example
(`cargo run -p xd-art-core --example convert -- in.png out.png [recipe.json]`) and doc
examples on the main entry points. `cargo publish -p xd-art-core --dry-run` passes:
28 files, 351.7 KiB (96.0 KiB compressed). The package leaves out `tests/`: the golden
files come from the original program and only make sense in the repository. Nothing is
published; whether to publish is the owner's call.

Dependencies of the engine (`cargo tree -p xd-art-core -e normal`, unique crates
including itself):

| features | crates |
|---|---|
| none (`--no-default-features`, the web build) | 21 |
| default (`parallel`, `gif`, `qoi`, `serde`) | 35 |
| all (adds `online-palettes`: ureq with rustls) | 56 |

Every direct dependency is used: `image` (decoding and most encoders), `png` (indexed
PNG, which `image` cannot write), `gif`, `rayon`, `serde_json`, `ureq`. The only
duplicate is `miniz_oxide` 0.8 and 0.9, both pulled in by `png` itself. `image` brings
`moxcms` and `pxfm` for color management whether or not they are needed; they cannot
be turned off in 0.25. Nothing was removed: there was nothing unused to remove.
`cargo-udeps` and `cargo-bloat` are not installed here, so the check was done by
reading the tree and searching the sources for each crate.

Release binaries on Windows: `xd-art.exe` (interface) 9.8 MB, `xd-art-cli.exe` 2.2 MB.
Most of the interface is gpui.
