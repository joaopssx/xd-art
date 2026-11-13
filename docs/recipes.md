# Recipes and presets

A recipe is every setting of a conversion as a small JSON file. The app saves and
loads them (export panel, or drop the file on the window), the command line tool reads
them with `--recipe` and writes them with `--save-recipe`, Lua scripts and the web page
use the same format, and the editor handoff export includes one.

```json
{
  "xd-art": 1,
  "sample": { "mode": "box", "width": 160, "keep_aspect": true },
  "adjust": { "contrast": 1.2, "gamma": 1.1 },
  "colors": { "palette": "game-boy", "metric": "redmean" },
  "dither": { "mode": "bayer4x4", "amount": 0.35 }
}
```

More in `examples/recipes/`.

## Keys

Only settings that differ from the defaults are written, and every key may be left
out. Names of modes are the same as on the command line (`xd-art-cli list ...`).

| section | key | value |
|---|---|---|
| `blur` | `radius` | number, 0 is off |
| | `compatible` | `true` for the original program's box blur |
| | `box_from` | radius from which the fast box approximation is used |
| `sample` | `mode` | `nearest`, `bilinear`, `bicubic`, `lanczos`, `box`, `cluster`, `edge` |
| | `width`, `height` | output size in pixels |
| | `keep_aspect` | `true`: the height follows the width |
| | `divide_x`, `divide_y` | instead of a size: divide the source size |
| | `offset` | `[x, y]`, sample offset in output pixels, -1 to 1 |
| | `edge_strength` | 0 to 1, for `edge` |
| `adjust` | `sharpen`, `brightness`, `contrast`, `saturation`, `hue`, `gamma` | numbers |
| | `tint` | `"rrggbb"` |
| | `tint_strength` | 0 to 1 |
| `colors` | `palette` | a built-in name, or a list of `"rrggbb"` colors |
| | `extract` | instead of a palette: `{ "count": 8, "method": "k-means", "locked": ["rrggbb"] }`, method `k-means`, `median-cut` or `balanced` |
| | `metric` | `rgb`, `weighted`, `redmean`, `cie76`, `cie94`, `ciede2000` |
| `dither` | `mode` | see `xd-art-cli list dithers` |
| | `amount` | 0 to 1 |
| | `alpha_threshold` | 0 to 255; pixels below it become transparent |
| | `target_colors` | color count for the median cut dither mode |
| `post` | `outline`, `inline` | palette index of the outline or inline color |
| | `cleanup` | 0 off, 1 removes fully surrounded single pixels, 2 also those with three matching neighbors that are not part of a line |
| | `merge` | palette colors closer than this (8-bit RGB distance) are merged, 0 is off |
| `output` | `scale` | whole number upscale when saving |

The dither mask is not part of a recipe: it is an image tied to one picture, while a
recipe is meant for any picture.

## Versions

`"xd-art"` is the format number, 1 today. The rules, so recipes keep working:

- New settings get new keys. An older xd-art ignores keys it does not know, so a newer
  recipe still loads there, minus the new settings.
- Missing keys take the default, so an older recipe loads in a newer xd-art unchanged.
- The number only goes up when an existing key changes meaning. A recipe with a higher
  number than the program knows is refused with a clear message instead of being read
  wrong.

Round trip tests in `crates/xd-art-core/src/recipe.rs` check that writing and reading
gives back the same settings.

## Presets of the original program

`--recipe` (and dropping the file in the app) also reads the JSON presets that
SLK_img2pixel saves. Their numbers for modes are translated to xd-art's names, and
their palette (a list of packed colors) becomes a color list.

## Style presets

Presets are named starting points built into the program: "start from" in the app,
`--preset <name>` on the command line, `xd-art-cli list presets` for the list.

| preset | what it sets up |
|---|---|
| `game-boy` | 160 pixels wide, four greens, ordered dithering |
| `pico-8` | 128x128 with the PICO-8 palette and light Floyd-Steinberg |
| `console-8bit` | 256 wide, a 32 color console style palette made for this project, 2x2 ordered dither |
| `dungeon` | 320 wide, dark browns and reds (palette made for this project), Sierra lite for gritty texture |
| `picocad` | PICO-8 palette with the picoCAD checker between close colors |
| `1-bit` | black and white with an 8x8 Bayer pattern |
| `posterize` | 12 colors taken from the image, no dithering, stray pixels cleaned up |
| `blocks-16`, `blocks-32`, `blocks-64` | square textures for block game resource packs, transparency kept, no dithering (see `docs/integration.md`) |

A preset names a style; it does not copy anything from the games or programs it is
reminiscent of. Where a palette has a known origin and license, `docs/credits.md` says
so; palettes made for this project are marked as such.
