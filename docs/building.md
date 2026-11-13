# Building

## Releases

When a version tag (`v0.1.0` and so on) is pushed, `.github/workflows/release.yml` builds
the app and the command line tool for Windows (x86-64), Linux (x86-64) and macOS (Apple
Silicon), and attaches them to the GitHub release of that tag with a `SHA256SUMS.txt`.
No release has been made yet, so the workflow has not run.

To use a release:

1. Download the archive for your system from the Releases page of the repository.
2. Check it: `sha256sum -c SHA256SUMS.txt --ignore-missing` on Linux and macOS, or
   `Get-FileHash xd-art-*.zip` in PowerShell and compare with the file.
3. Unpack it anywhere and run `xd-art` (the app) or `xd-art-cli` (the command line
   tool). Nothing needs installing; `integration/` has optional launcher and right
   click menu files (`docs/integration.md`).

The Windows binaries need nothing else. The Linux app needs the usual desktop
libraries listed below. The macOS build is not signed, so macOS asks before the first
start (right click, Open).

## From source

You need Rust. The toolchain version is pinned in `rust-toolchain.toml`; with rustup
installed, `cargo` picks it up on its own.

```sh
cargo build --release                 # everything
cargo build --release -p xd-art-cli   # just the command line tool
cargo run --release -p xd-art-gui     # the desktop app
```

The binaries end up in `target/release/`: `xd-art` (GUI) and `xd-art-cli`. The browser
build has its own steps in `docs/web.md`.

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
