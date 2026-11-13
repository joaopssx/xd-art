# Command line

`xd-art-cli` does everything the engine does, without a window. It is a separate
binary so scripts and servers do not carry the GUI.

```sh
xd-art-cli convert photo.jpg -o sprite.png -w 64 --keep-aspect -p pico-8 -d floyd -s 4
xd-art-cli convert anim.gif -o small.gif --extract 12 --gif-palette shared
xd-art-cli convert photo.jpg --suggest
xd-art-cli info sprite.png anim.gif
xd-art-cli list palettes
```

## Commands

| command | what it does |
|---|---|
| `convert <input> -o <output> [options]` | one conversion; the output format comes from the extension |
| `info <file>...` | prints size (and frame count for GIFs) from the file header |
| `list <what>` | `palettes`, `presets`, `samplers`, `metrics` or `dithers` |
| `--version`, `--help` | |

Input formats: PNG, JPEG, BMP, TGA, QOI, GIF. Output formats: PNG, BMP, TGA, QOI,
GIF. A GIF in and a GIF out keeps the animation; a GIF in and anything else out
converts the first frame. JPEG is not offered as output on purpose, its compression
smears exactly the hard edges pixel art is made of.

## Options

Options apply in the order given, so `--preset game-boy -w 80` starts from the preset
and then changes the width. `--preset` and `--recipe` replace everything set before
them.

### Size and sampling

| option | default | |
|---|---|---|
| `-w`, `--width <px>` | 64 | target width |
| `-h`, `--height <px>` | 64 | target height (`-h` is height, help is `--help`) |
| `--keep-aspect` | off | height follows the width and the source aspect ratio |
| `--divide <n>` or `<nxm>` | | divide the source size instead of a fixed size |
| `--sample <mode>` | `nearest` | `nearest`, `bilinear`, `bicubic`, `lanczos`, `box`, `cluster`, `edge` |
| `--offset <x,y>` | `0,0` | sample position shift in output pixels, -1 to 1 |
| `--edge-strength <0..1>` | 0.6 | how hard `edge` keeps edges |

### Prepare and adjust

| option | default | |
|---|---|---|
| `--blur <radius>` | 0 | blur before sampling |
| `--blur-compat` | off | the original program's box blur instead of a gaussian |
| `--sharpen <amount>` | 0 | unsharp mask after sampling |
| `--brightness <-1..1>` | 0 | |
| `--contrast <x>` | 1 | |
| `--saturation <x>` | 1 | 0 is grayscale |
| `--hue <degrees>` | 0 | |
| `--gamma <x>` | 1 | |
| `--tint <rrggbb>` | `ffffff` | multiplied into every pixel |
| `--tint-strength <0..1>` | 1 | |

### Colors

| option | |
|---|---|
| `-p`, `--palette <name or file>` | built-in name (`list palettes`) or a `.gpl`, `.hex`, `.pal` or image file |
| `--extract <n>` | build an n color palette from the image |
| `--method <name>` | `k-means` (default), `median-cut` or `balanced` |
| `--lock <rrggbb>` | a color the extracted palette must keep; repeat for more |
| `--lospec <name or link>` | download from lospec.com; only in builds with the `online-palettes` feature |
| `--metric <name>` | `rgb`, `weighted`, `redmean` (default), `cie76`, `cie94`, `ciede2000` |

### Dithering

| option | default | |
|---|---|---|
| `-d`, `--dither <mode>` | `none` | `list dithers` for every mode |
| `--amount <0..1>` | 0.2 | strength of ordered dithering |
| `--alpha-threshold <0..255>` | 128 | pixels below become transparent |
| `--target-colors <n>` | 8 | boxes for `median-cut` |
| `--mask <image>` | | grayscale image, white is full dithering, black none |

### Post-processing and output

| option | default | |
|---|---|---|
| `--outline <index>` | | palette index drawn around shapes |
| `--inline <index>` | | palette index drawn on the inner edge |
| `--cleanup <0..2>` | 0 | remove isolated pixels |
| `--merge <distance>` | 0 | merge palette colors closer than this |
| `-s`, `--upscale <n>` | 1 | repeat every pixel n times |
| `--scanlines <0..1>` | 0 | darken one row per scaled pixel row |
| `--crt <0..1>` | 0 | screen curvature and vignette |
| `--gif-palette <mode>` | `shared` | `shared` or `per-frame` for animated output |
| `--handoff <dir>` | | also write an indexed PNG, `.gpl`, `.hex` and `.json` there |

### Settings files

| option | |
|---|---|
| `--preset <name>` | start from a style preset (`list presets`) |
| `--recipe <file>` | start from a recipe; presets saved by SLK_img2pixel work too |
| `--save-recipe <file>` | write the settings that were used |

Recipes are described in [recipes.md](recipes.md).

### Other

| option | |
|---|---|
| `--suggest` | print a suggested target size and why, then exit |
| `--json` | one JSON object per file on stdout instead of text |
| `-q`, `--quiet` | print nothing on success |
| `-f`, `--force` | replace output files that exist |
| `-j`, `--threads <n>` | worker threads; the default is one per core |

## Exit codes

| code | meaning |
|---|---|
| 0 | everything worked |
| 1 | a conversion failed (file not found, broken image, output exists without `--force`) |
| 2 | usage error: unknown option, bad value, missing file name |

Errors go to stderr as one short line, for example
`could not access photo.png: No such file or directory (os error 2)`.
