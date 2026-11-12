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
