# Lua scripting

Scripts are for what one command line call cannot do on its own: loops over files
with logic in between, settings that depend on the image, several outputs from one
input, sprite sheets from frames.

```sh
xd-art-cli run script.lua [arguments]...
xd-art-cli --script script.lua [arguments]...   # the original program's spelling
```

The arguments are in the global `arg` table (`arg[1]`, `arg[2]`, ...; `arg[0]` is the
script). A script that is not found as given is looked up in `examples/lua/`.

Lua is 5.4, compiled into the binary. The whole standard library is there (`io`, `os`,
`string`, `table`, `math`), so scripts can read and write files like any local
program. Only run scripts you trust, as with any other program.

Nothing about Lua exists until a script runs: the CLI creates the Lua state for `run`
and drops it afterwards, and the GUI does not load it at all.

## The `xd` table

| function | |
|---|---|
| `xd.load(path)` | loads an image, returns an image object |
| `xd.convert(image, settings)` | converts, returns a new image object |
| `xd.process(input, output, settings)` | file to file, GIF to GIF keeps the animation; returns `true`, or `false` and the error |
| `xd.files(folder, pattern)` | sorted list of files in `folder` whose name matches `pattern` (`*`, `?`) |
| `xd.sheet(images, columns, spacing)` | packs images of the same size into one; `columns` 0 or nil is one row |
| `xd.preset(name)` | the settings of a preset, as a table |
| `xd.recipe(path)` | the settings of a recipe file (or an SLK_img2pixel preset), as a table |
| `xd.palettes()`, `xd.presets()` | lists of names |
| `xd.time()` | seconds since the script started, for timing |
| `xd.version` | the xd-art version |

Image objects have `:width()`, `:height()`, `:pixel(x, y)` (four values, r g b a, or
nothing outside the image), `:save(path)` and `:upscale(n)`.

### Settings

`settings` can be:

- `nil`: the defaults;
- a string: the path of a recipe file;
- a table shaped like a recipe ([recipes.md](recipes.md)), with any part left out:

```lua
local out = xd.convert(img, {
  sample = { mode = "box", width = 64, keep_aspect = true },
  adjust = { saturation = 1.2 },
  colors = { palette = "pico-8", metric = "redmean" },
  dither = { mode = "floyd" },
  output = { scale = 4 },
})
```

`xd.preset` and `xd.recipe` return the same shape, so a script can start from one and
change a few values. They only contain what differs from the defaults.

### Errors

A Lua error stops the script with the file name and line:

```text
runtime error: convert.lua:12: could not access photo.png: No such file or directory (os error 2)
```

The exit code is then 1. `xd.process` does not raise errors, it returns them, so batch
scripts can carry on past a broken file.

## The `img2pixel` table

Scripts written for SLK_img2pixel run as they are. The table has the original's fields
(`size_absolute_x`, `dither_mode`, `color_dist`, ...), the `DITHER` and `COLORDIST`
constants, `process(input, output)`, `load_preset(path)`, `get_palette_color(i)` and
`set_palette_color(i, 0xRRGGBB)`. Writing a field that does not exist is an error, as in
the original. The differences are listed in [compat.md](compat.md#lua-scripts).

## Examples

Every script in `examples/lua/` takes its paths as arguments. All of them were run
against test images while writing this.

| script | |
|---|---|
| `convert_folder.lua <in> <out> [recipe]` | every PNG in a folder, with an optional recipe, with a count and the time taken |
| `compare_dithers.lua <image> <out> [width]` | one file per dither mode, to choose by eye |
| `sprite_sheet.lua <folder> <pattern> <sheet.png> [columns]` | numbered frames into one sheet with the same palette |
| `apply_recipe.lua <recipe> <image> <out.png>` | a recipe, with the width picked from the image size |
| `original_api.lua <image> <out.png>` | the same job written against `img2pixel` |

Two recipes to try them with are in `examples/recipes/`.
