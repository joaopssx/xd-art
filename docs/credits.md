# Credits

## SLK_img2pixel

xd-art started as a port of SLK_img2pixel, part of
[SoftLK-tools](https://github.com/Captain4LK/SoftLK-tools), written by
Lukas Holzbeierlein ([Captain4LK](https://github.com/Captain4LK)).

The SoftLK-tools code (everything except its `external` folder) is dedicated to the
public domain under CC0 1.0. That is what made this port possible without asking for
anything, but I still want to say it clearly: the hard part, figuring out which
algorithms give good pixel art and how to tune them, was done there. Thank you, Lukas.

None of the original source files are copied into this repository. I read them and
rewrote the algorithms in Rust.

## What came from the original

- The overall pipeline: blur, sample down, sharpen and adjust colors, then map to a
  palette with or without dithering.
- The sampling modes: nearest, bilinear, bicubic, lanczos and cluster.
- The blur (a three pass box blur with a fractional radius) and unsharp-mask sharpening.
- Hue, saturation, contrast, brightness, gamma and tint adjustments.
- The color distance metrics: RGB euclidean, weighted RGB, redmean, CIE76, CIE94 and
  CIEDE2000.
- The dithering modes: bayer 2x2, 3x3, 4x4, 5x5 and 8x8, cluster 4x4 and 8x8,
  Floyd-Steinberg (two variants), Stucki, Burkes, Sierra, Sierra two-row, Sierra lite,
  picoCAD and median-cut assignment.
- k-means palette generation, with k-means++ seeding.
- Inline and outline drawing.
- Palette files in .gpl, .hex, .pal and .png.
- GIF import and export with a shared palette.
- Lua scripting and JSON presets, in spirit. The API is close but not identical,
  see [compat.md](compat.md).

## What was redone

- The interface. The original uses its own SDL based toolkit. xd-art uses GPUI, the UI
  framework of the Zed editor, with a dark theme and a layout centered on the image.
- The image buffers. The original works on 15 bit per channel RGBA. xd-art keeps
  that working format inside the pipeline, but stores loaded images as packed RGB with an
  optional alpha plane.
- Settings, presets and the command line tool.

## What is new

Everything else: the stage cache and cancellation, the exact gaussian blur, the color
lookup structures, the parallel dithering, the GIF frame editor, the retouch brush,
recipes, exporters, the web build and the rest. Each part has its own document in this
folder.

## Other material

Fonts, palettes and any other third party data used by xd-art are listed here with their
source and license as they are added.

### Built-in palettes

Color values are facts, not creative works, but I still only ship palettes whose origin
I can state. The source line of each one is also in `palette.rs` and shown in the
palette picker.

| name | colors | source |
|---|---|---|
| `pico-8` | 16 | the published palette of the PICO-8 fantasy console (Lexaloffle) |
| `game-boy` | 4 | the four greens commonly used to emulate the original Game Boy screen; the hardware had no RGB values, these are a widespread convention |
| `cga` | 16 | IBM CGA RGBI colors, with color 6 as brown like the IBM 5153 monitor showed it |
| `1-bit` | 2 | black and white |
| `gray-4`, `gray-16` | 4, 16 | evenly spaced grays, computed |
| `dungeon-16` | 16 | made for xd-art |
| `console-32` | 32 | made for xd-art, in the spirit of 8-bit console palettes; not a copy of any console's |
| `web-216` | 216 | the web safe color cube, computed |
| `rgb-332` | 256 | 3-3-2 bit truecolor, computed |

The original program starts with the DawnBringer 32 palette. I left it out because I
could not find a license statement from its author. It is easy to load from a `.gpl` or
`.hex` file, or through the optional Lospec import.
