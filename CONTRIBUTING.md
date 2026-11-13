# Contributing

Issues and pull requests are welcome: bugs, results that look wrong next to the
original program, missing palettes with a clear license, translations, gallery entries.

## Building and checking

`docs/building.md` has what each system needs. Before sending a change:

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

The CI runs the same on Linux and Windows, plus the browser build and a speed check
(`docs/performance.md`).

## How the code is written

- Plain, direct Rust. Comments say why, not what. No `unsafe` unless there is no other
  way, and then with a `// SAFETY:` comment (the lints enforce it).
- Engine code returns `Result` with a message a user can act on; it never panics on bad
  input.
- Anything that changes pixels needs a test. For options the original program has, the
  golden tests compare against its output (`scripts/gen-goldens.sh` makes the reference
  files from its sources).
- A change that could be slower comes with a benchmark number from
  `cargo bench -p xd-art-bench`, measured, not guessed.
- Interface text goes through `tr()` (see below).

## Commits

Conventional Commits in English: `feat(core): add ...`, `fix(gui): ...`, `docs: ...`,
imperative, at most 72 characters, no period. A short body when the reason is not
obvious. One logical change per commit.

## Decisions

When you choose between options that someone might question later, add a short entry
to `docs/decisions.md`: what was chosen and why.

## Adding a language

The interface text lives in `crates/xd-art-text`. English is the language of the
sources: every visible string is written in English and passed through `tr()` (or
marked with `mark()` where it is stored and shown later). A language is one table that
maps that English text to the translation.

1. Copy `crates/xd-art-text/src/pt.rs` to a file named after the language code, for
   example `es.rs`, and replace the right-hand side of every pair. Keep the left side
   exactly as it is, including `{0}`, `{1}` and the spacing. The numbered placeholders
   can move around inside the sentence if the language puts things in another order.
2. In `crates/xd-art-text/src/lib.rs`: add the module, a variant to `Lang`, its name in
   its own language in `Lang::name`, its code in `Lang::code` and `Lang::from_code`,
   its table in `Lang::table`, and a number for it in `set` and `current`.
3. The desktop app stores the choice in two bits of the config flags (`config.rs`),
   which have room for one more language; past that the config format needs a new
   version. The language shortcut (Ctrl+Shift+L) cycles through `Lang::ALL`.
4. Add the help text of the command line tool (`HELP_PT` in
   `crates/xd-art-cli/src/main.rs` is the model) and pick it in `help()`.
5. Run `cargo test -p xd-art-text`. It checks that the table is sorted, that every
   translation keeps the placeholders of its English text, and that every string the
   app and the command line tool pass to `tr()` or `mark()` has a translation.

Some text stays in English on purpose: names that are also values on the command line
and in recipe files (`nearest`, `bayer4x4`, `redmean`, palette and preset names), the
descriptions of built-in palettes and presets, messages from the engine itself (decode
errors, invalid settings), Lua errors, the JSON output of the command line tool, and the
developer timing overlay.

Write the translation the way people who speak the language talk about images and
software. Short beats literal: a button has room for one or two words.
