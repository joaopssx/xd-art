# Working with other programs

How xd-art fits next to a file manager, a pixel art editor and a game engine. Nothing
here is installed by xd-art itself; the files in `integration/` are for you to copy,
read and import by hand.

## Opening files

`xd-art photo.png` opens the image and shows it converted with the last settings used.
Several images ask whether to convert them all into a folder (a batch). A `.gpl`,
`.hex` or `.pal` file is applied as the palette, a `.json` recipe as the settings, in
any combination with images. The same happens when files are dropped on the open
window.

Dropping files on the program itself depends on the system:

| system | dropping files on the program |
|---|---|
| Windows | works on `xd-art.exe` or a shortcut to it: Explorer starts it with the paths as arguments |
| Linux | works on the launcher from `integration/linux/xd-art.desktop` in file managers and docks that support it (`Exec` uses `%F`); dropping on the bare binary does nothing in most file managers |
| macOS | not supported: Finder only drops files on an `.app` bundle, and xd-art does not ship one yet. Use the command line, or drop the files on the open window |

With `--resident` (see `docs/performance.md`) a second launch hands its files to the
window already open instead of starting again.

## Linux launcher

`integration/linux/` has a desktop entry with an extra action, "Convert to pixel art
next to the file", which runs `xd-art-cli batch --next-to-input` on the selected files:
each result is written beside its source as `<name>-px.png`.

```sh
install -Dm644 integration/linux/xd-art.desktop ~/.local/share/applications/xd-art.desktop
install -Dm644 integration/linux/xd-art.svg ~/.local/share/icons/hicolor/scalable/apps/xd-art.svg
update-desktop-database ~/.local/share/applications   # where available
```

`xd-art` and `xd-art-cli` have to be on `PATH` (for example in `~/.local/bin`). Whether
the extra action shows up in the right click menu of a file depends on the file manager:
GNOME Files and KDE Dolphin list desktop actions under "Open With" or in the launcher's
own menu, not always on the file. The desktop entry was written to the Desktop Entry
specification but not tested on a Linux desktop.

## Windows context menu

`integration/windows/xd-art-context-menu.reg` adds two entries to the right click
menu of images: "Open in xd-art" and "Convert to pixel art". They are written for the
current user only, so no administrator rights are needed.

1. Open the file in a text editor and replace `C:\\Program Files\\xd-art` with the folder
   that holds `xd-art.exe` and `xd-art-cli.exe` (backslashes stay doubled).
2. Double click the file and confirm.

`xd-art-context-menu-remove.reg` removes both entries and nothing else.

Limitations, all from how Windows does this:

- The entries are registered for files Windows considers images (`.png`, `.jpg`, `.bmp`,
  `.gif` and others). `.tga` and `.qoi` usually are not, so they do not get the menu.
- "Convert to pixel art" runs once per selected file, and a console window appears
  briefly. If `<name>-px.png` already exists, the conversion stops instead of replacing
  it, and the message is gone with the window. Open the folder in a terminal and run
  `xd-art-cli batch --next-to-input -f <files>` to see messages or to overwrite.
- On Windows 11 the entries are under "Show more options".

The `.reg` files were checked by reading them; they were not imported on this machine.

## Handoff to a pixel art editor

"For an editor" in the export panel, or `--handoff <dir>` on the command line, writes
four files with the same name:

| file | what it is |
|---|---|
| `<name>.png` | indexed PNG (color type 3): Aseprite, GIMP and most editors open it with the palette in order. Transparent pixels use one extra palette slot (or slot 0 when the palette is full) |
| `<name>.gpl` | the palette for GIMP, Inkscape, Krita, Aseprite |
| `<name>.hex` | the palette as one `rrggbb` per line, the format Lospec and many tools take |
| `<name>.json` | size, colors and the recipe that made the image, so it can be made again |

The JSON file:

```json
{
  "xd-art-handoff": 1,
  "image": "cat.png",
  "width": 64,
  "height": 64,
  "colors": ["000000", "1d2b53", "..."],
  "transparent": false,
  "recipe": { "xd-art": 1, "dither": { "mode": "floyd" } }
}
```

`recipe` is a normal recipe (`docs/recipes.md`); saving it as its own file and passing
it to `--recipe` repeats the conversion.

## Sprite sheets for game engines

The export panel's "sprite sheet" and `export::sprite_sheet` in the engine pack frames
of one size into a grid, one row or one column, with optional transparent spacing
between frames and around the sheet. Next to `<name>.png` come `<name>.json` and
`<name>.gpl` (every color used by any frame).

```json
{
  "xd-art-sheet": 1,
  "image": "walk.png",
  "frame_width": 32,
  "frame_height": 32,
  "columns": 8,
  "rows": 2,
  "spacing": 1,
  "frames": [
    { "x": 1, "y": 1, "w": 32, "h": 32, "duration_ms": 100 },
    { "x": 34, "y": 1, "w": 32, "h": 32, "duration_ms": 100 }
  ]
}
```

Frames are listed in order, left to right and top to bottom. `x` and `y` are the top
left corner in the sheet, in pixels. `duration_ms` is 0 for still images. The format is
xd-art's own; it is not the format of any particular engine, and no engine was tested
with it. It is plain enough to read in a few lines of any scripting language, or to
convert to an engine's own atlas format. The `"xd-art-sheet"` number goes up only if a
field changes meaning.

## Block game textures

Three presets make square textures for block game resource packs (Minecraft style):
`blocks-16`, `blocks-32` and `blocks-64`. They use box sampling, the 24 color
`blocks-24` palette made for this project (stone, dirt, wood, leaves, water and ore
tones), a hard alpha cut at 128 so leaves and glass keep crisp edges, and light cleanup
of single stray pixels. No textures or palettes from any game are included.

A whole folder at once, with names resource packs accept:

```sh
xd-art-cli batch textures/ -r -o pack/ --preset blocks-16 --texture-names
```

`--texture-names` writes lowercase ASCII names with the size appended: `Oak Log.png`
becomes `oak_log_16.png`. In the interface, pick a `blocks` preset under "start from"
and use "block texture" in the export panel, which uses the same naming.
