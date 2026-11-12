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
