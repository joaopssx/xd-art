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
