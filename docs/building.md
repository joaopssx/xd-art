# Building

You need Rust. The toolchain version is pinned in `rust-toolchain.toml`; with rustup
installed, `cargo` picks it up on its own.

```sh
cargo build --release                 # everything
cargo build --release -p xd-art-cli   # just the command line tool
cargo run --release -p xd-art-gui     # the desktop app
```

The binaries end up in `target/release/`: `xd-art` (GUI) and `xd-art-cli`.

For day to day work on the interface, `cargo run --profile fast -p xd-art-gui` gives a
build that is almost as fast as release but links in a fraction of the time.

## Windows

Nothing beyond Rust and the MSVC build tools (installed with Visual Studio or the
"Build Tools for Visual Studio"). The C runtime is linked statically, so the resulting
`.exe` runs on a machine without the Visual C++ redistributable.

This is the platform I develop on (Windows 11, rustc 1.95.0).

## Linux

The engine and the CLI need nothing from the system.

The GUI is a different story. gpui talks to the display server and the GPU through
system libraries. I found the exact list by building in Ubuntu 26.04 under WSL:
`cargo check -p xd-art-gui` works with no extra packages, but linking failed until these
are present:

```sh
sudo apt install pkg-config libxcb1-dev libxkbcommon-dev libxkbcommon-x11-dev
```

At run time gpui also loads Vulkan, Wayland and fontconfig, which a normal desktop
install already has (`libvulkan1`, `libwayland-client0`, `libfontconfig1`). It needs a GPU
driver with Vulkan support.

Because of these libraries, a fully static Linux build of the GUI is not possible. The
CLI is a normal Rust binary that only depends on libc.

I have not run the GUI on a Linux desktop. The CI builds it on Ubuntu with the packages
above installed.

### libc pin

gpui 0.2.2 depends (through its HTTP client) on `xattr 0.2.3`, which uses
`libc::ENOATTR`. libc removed that constant for Linux in 0.2.190, so the lock file pins
`libc` to 0.2.189. Without the pin the Linux build fails in `xattr`. Running
`cargo update` without care will bring the problem back.

## macOS

gpui supports macOS through Metal. I have no Mac to test on, so this is unverified.
