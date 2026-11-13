# Roadmap

## What exists

- The engine: every option of SLK_img2pixel with matching output (`docs/compat.md`),
  plus edge-preserving sampling, palette extraction with locked colors, region masks
  for dithering, outline, inline and stray pixel cleanup, GIF in and out, size
  suggestions, scanline and CRT export effects, presets and recipe files, sprite sheets,
  editor handoff, block game textures and an optional Lospec palette import.
- The desktop app on Windows (built and run), Linux (type-checked under WSL; the CI
  workflow builds and tests it, but has not run yet; never run on a desktop) and macOS (only in the release workflow, not built yet), with live
  preview, comparison tools, history, retouching, a GIF frame editor,
  English and Portuguese.
- The command line tool with batch mode, JSON output and Lua scripts.
- A browser build of the engine with a small page.
- Benchmarks, a speed check in CI, a comparison with the original program.

## Planned

- A Web Worker for the web page, so big conversions do not block it.
- An app bundle for macOS, so files can be dropped on the icon.
- A Linux run of the app on a real desktop, and fixes for whatever that finds.
- An `.ico` resource for the Windows executable.
- Faster first frame: about 240 ms of the startup is spent inside gpui before xd-art
  gets control (`docs/performance.md`), and where it goes has not been measured yet.

## Left out on purpose

- **A GPU path.** Measured and decided against (`docs/decisions.md`): setup costs more
  than it saves, and results would differ slightly from the CPU ones.
- **A light theme.** The interface is built around dark surfaces next to the artwork.
- **Network features beyond Lospec import.** No update checks, no telemetry, no
  accounts. Lospec is off by default and only contacted when asked.
- **A drawing program.** The retouch tools fix a few pixels; real editing belongs in a
  pixel art editor, which the handoff export feeds.
- **Assets from games.** Presets name a style; no palettes, textures or sprites from
  games or other programs are included unless their license allows it
  (`docs/credits.md`).
- **A changelog file.** The commit history is the record.
