# xd-art-core

The engine behind [xd-art](https://github.com/joaopssx/xd-art). It takes an image and
turns it into pixel art: blur, sample down, adjust colors, map to a palette with or
without dithering, then post-process.

It has no user interface code and can be used on its own.

## Features

| feature | default | what it does |
|---|---|---|
| `parallel` | yes | splits work across threads with rayon; output is identical either way |
| `gif` | yes | animated GIF import and export |
| `qoi` | yes | QOI image format |
| `serde` | yes | recipe files and JSON output |
| `online-palettes` | no | download palettes from Lospec |

License: MIT.
