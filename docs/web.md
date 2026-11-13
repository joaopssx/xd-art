# Web build

`crates/xd-art-wasm` compiles the engine to WebAssembly, and `web/` is a plain page
(HTML, CSS and JavaScript, no framework, no build step) around it: choose an image,
change the options, download the PNG. The image never leaves the browser.

## Build

Once:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.130 --locked
```

The CLI version has to match the `wasm-bindgen` version in
`crates/xd-art-wasm/Cargo.toml` exactly.

Then, from the repository root:

```sh
cargo build -p xd-art-wasm --target wasm32-unknown-unknown --profile wasm
wasm-bindgen --target web --out-dir web/pkg target/wasm32-unknown-unknown/wasm/xd_art_wasm.wasm
```

The `wasm` profile is the release profile without `strip`, because wasm-bindgen needs
a section that stripping removes. `web/pkg` is not committed.

## Run

Browsers do not load modules from `file://`, so serve the folder:

```sh
python -m http.server 8000 --directory web
```

and open <http://localhost:8000>. Any static file server works, as long as it sends
`.wasm` files as `application/wasm`.

## What is in it

The engine is built with `--no-default-features` plus `serde` (for recipes): no
threads, no GIF, no QOI, no network. The page builds a recipe from its controls (the
same JSON format as `docs/recipes.md`) and calls:

| function | does |
|---|---|
| `convert(bytes, recipe)` | decodes PNG, JPEG, BMP or TGA, converts, returns PNG bytes |
| `default_recipe()` | the recipe of the default settings |
| `names(kind)` | menu entries for `"palettes"`, `"samplers"`, `"dithers"`, `"metrics"` |

Errors come back as JavaScript exceptions with the engine's message.

There is no clock in `wasm32-unknown-unknown`: `std::time::Instant::now` panics there.
The pipeline measures stage times with it, and the first build crashed on the first
conversion for that reason; the pipeline now skips the measurement on that target and
reports zero.

## What was checked

- `cargo test -p xd-art-wasm` runs the conversion code natively.
- The built module was loaded in Node.js 26 (`initSync` with the `.wasm` bytes):
  `convert` turned a 1920x1080 PNG into a 64x64 PNG with the default recipe in 39 ms,
  pixel for pixel the same as `xd-art-cli convert` with the same settings, and a broken
  file gave a readable error.
- The page was opened in headless Chrome from a local server: the module loaded and
  the menus filled from it.

Not checked: choosing a file, converting and downloading by hand in a browser. Those
steps only use standard browser APIs around the calls above, but nobody has clicked
through them yet.
