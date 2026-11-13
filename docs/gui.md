# The desktop app

`xd-art` (the `xd-art-gui` crate) is one window: settings on the left, the image in the
middle, information and tools on the right, the palette under the image, a status bar
and the `github/joaopssx` link at the bottom.

## Opening images

- Ctrl+O, dropping files on the window, or `xd-art photo.png` on the command line.
- Several images at once offer to convert them all into a folder with the current
  settings, or to open the first one.
- Dropping a palette (`.gpl`, `.hex`, `.pal`) applies it; dropping a recipe (`.json`)
  applies its settings.
- With no image, the window lists recent files (read after the window is up).
- The last settings come back on the next start, so an image opened from the command
  line is converted the way the previous one was.
- Images over 1.5 megapixels are shown from a reduced copy first, then exactly.

## Settings panel

Sections in the order the conversion runs: Input, Prepare, Sampling, Color, Dithering,
Post, Output. A section only shows options that do something with the current settings
(no Lanczos options with nearest sampling, no dither amount with dithering off).
Sections fold with a click on their title; both side panels hide with Ctrl+[ and Ctrl+]
and can be resized by dragging their edge.

"Start from" in Input applies a style preset (Game Boy, PICO-8, 1-bit, block textures
and others; `xd-art-cli list presets` shows them all).

Sliders show their value with fixed-width digits. Click a slider and press Enter to
type a value (Enter applies, an invalid value is refused), double click to reset it,
arrow keys step it, Shift with an arrow steps ten times further. While a slider is
dragged the preview uses a faster approximation; the exact result follows as soon as
it stops.

Hovering a dither mode in its menu shows a small preview of it on the current image.

## The image

| | |
|---|---|
| mouse wheel | zoom around the pointer |
| middle drag, or Space and drag | move the image |
| Ctrl+0 / Ctrl+1 | fit to the window / 100% |
| Ctrl+= / Ctrl+- | zoom in / out |
| Ctrl+G | pixel grid (only drawn when pixels are big enough) |
| Ctrl+B | before and after: a divider you can drag, then the source alone, then off |
| F11 | only the image, full screen; Escape leaves |

Pixels are drawn with nearest neighbor scaling on the CPU at the screen's own
resolution, so they stay square at every zoom and on high DPI screens.

## Palette strip

Under the image, one swatch per palette color.

- Hover: hex value and how many pixels use the color.
- Click: edit the color as hex (Enter applies, Escape cancels).
- Drag a swatch onto another to reorder.
- Right click: lock or unlock (locked colors survive palette extraction), duplicate,
  remove, sort by lightness or hue.

## Comparing

- **Dither gallery** (Ctrl+D): every dither mode on the current image, computed in the
  background in the order they appear; click one to use it.
- **History** (right panel): every change with a thumbnail and a short name; click to
  go back. Ctrl+Z and Ctrl+Shift+Z undo and redo.
- **Variations** (Ctrl+E): up to four versions side by side with zoom and position
  shared. Ctrl+Shift+E keeps the current settings as a variation; "sizes 32, 64, 128"
  adds the same settings at three sizes. Mark one as favorite, use its settings, or
  export it.

## Retouching

In the right panel, under Retouch:

- **Pencil** paints with a palette color: pick it by clicking a swatch, or with the
  **picker** on the image. **Eraser** gives pixels back to the conversion.
- **No dither / dither** paint the dithering mask: where it is black there is no
  dithering, where it is white there is full dithering.
- Brush sizes 1, 2, 4 and 8 pixels. Ctrl+Z takes back pencil strokes while the pencil
  or the eraser is active; mask strokes are in the normal history. Escape puts the tool
  away.
- Painted pixels stay when settings change, are always palette colors, and are part of
  every still export. "Save with image" writes them and the mask as
  `name.png.retouch.png` and `name.png.mask.png` next to the image; opening the image
  loads them back.

The tools are off for animated GIFs.

## GIF frames

Opening an animated GIF shows its frames under the image. Click a frame to see it
converted, Shift click to select a range. Skip (kept but not exported), remove,
duplicate, move left or right, make longer or shorter, and play the converted
animation with each frame's timing.

## Export (Ctrl+S)

- **Scale** 1x to 8x for every export.
- **PNG** of the result, optionally with scanlines or a CRT curve.
- **GIF**: the animation without skipped frames, or a still GIF.
- **Sprite sheet**: all frames in one PNG as a grid, a row or a column, with or without
  1 pixel spacing, plus a JSON file with positions and durations and a `.gpl` palette.
- **Block texture**: a PNG named the way resource packs expect, like `oak_log_16.png`.
- **For an editor**: indexed PNG, `.gpl`, `.hex` and a JSON file with the recipe.
- **Recipes**: save the settings as a small JSON file, or load one (dropping works too).
- **Palette from lospec.com**: type a palette name or link and press Enter. Only in
  builds with the `online-palettes` feature, and only when you ask for it; nothing else
  in the app uses the network.

File formats are described in `docs/integration.md` and `docs/recipes.md`.

## Looks

- Ctrl+T cycles four dark themes: dark, darker, midnight, plum.
- Ctrl+Shift+T takes the accent color from the image's palette, picking a color with
  enough contrast against the background or keeping the default one.
- Panels fade in over 120 ms. Ctrl+Shift+M turns transitions off; on Windows they are
  also off when "Show animations in Windows" is off. Nothing moves while the app is idle.

## Commands and shortcuts

Ctrl+K opens the command palette: type part of a command (in English or the current
language) and press Enter. Ctrl+/ lists every shortcut. On macOS, Cmd takes the place
of Ctrl.

| keys | command |
|---|---|
| Ctrl+O | open an image |
| Ctrl+[ | show or hide the settings panel |
| Ctrl+] | show or hide the information panel |
| Ctrl+0 | fit the image into the view |
| Ctrl+1 | show the image at 100% |
| Ctrl+= | zoom in |
| Ctrl+- | zoom out |
| Ctrl+G | show or hide the pixel grid |
| Ctrl+B | before and after: divider, source only, off |
| Ctrl+D | dither gallery |
| Ctrl+Z | undo |
| Ctrl+Shift+Z | redo |
| Ctrl+E | variations side by side |
| Ctrl+Shift+E | keep current settings as a variation |
| Ctrl+K | command palette |
| Ctrl+/ | list every shortcut |
| Ctrl+T | next theme |
| Ctrl+Shift+T | accent color from the image on or off |
| F11 (Cmd+Ctrl+F) | image only, full screen |
| Ctrl+Shift+M | transitions off or on |
| Ctrl+S | export, recipes and palette import |
| Ctrl+Shift+D | startup and conversion times |
| Ctrl+Shift+R | stay resident after closing, on or off |
| Ctrl+Q | quit |
| Ctrl+Shift+L | language: English or Portuguese |

All of them are defined in one table, `crates/xd-art-gui/src/actions.rs`.

## Language

English or Brazilian Portuguese, from the system at first (`XD_ART_LANG=en` or `pt`
overrides it); Ctrl+Shift+L switches and the choice is saved.

## Status bar

Source size and result size, colors used, dither mode, the time of the last conversion
and of each stage, "preview" while the result is an approximation (click it for the
exact one), and the memory held by the image, the result and the stage cache.

## Resident mode

`xd-art --resident`, or Ctrl+Shift+R for the next starts, keeps the first window
around: closing it minimizes it and frees the image, and later launches hand their
files to it and exit in about 40 ms instead of starting again. Ctrl+Q quits for real.
Details and memory numbers in `docs/performance.md`.

## For developers

- `XD_ART_TRACE=1` prints startup steps and stage times to stderr; `XD_ART_TRACE=file`
  appends them to a file. Ctrl+Shift+D shows them over the image.
- `XD_ART_TRACE_EXIT=frame` or `image` quits after the first frame or the first shown
  image, for timing (`scripts/measure-startup.sh`).

## Files

The preferences file is a few hundred bytes: `%APPDATA%\xd-art\config.bin` on Windows,
`~/Library/Application Support/xd-art/config.bin` on macOS,
`$XDG_CONFIG_HOME/xd-art/config.bin` or `~/.config/xd-art/config.bin` elsewhere. It is
read once at startup and written in the background after changes settle; a damaged
file is replaced by the defaults. Resident mode adds `resident.txt` next to it, and
Lospec downloads are cached in a `lospec` folder there.

## What was not checked by hand

The app was built and run on Windows 11, and screenshots were taken of its main states
(empty window, an open image, the panels, the retouch section, the Portuguese
interface). Mouse and keyboard interactions (dragging files onto the window, painting,
the file dialogs, the export buttons, the shortcuts) were written and compiled but not
exercised by hand; the parts under them (export jobs, retouch layers and strokes,
resident handoff, config file, shortcut table) have tests. Linux and macOS builds were
not run.
