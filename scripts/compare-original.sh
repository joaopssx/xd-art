#!/bin/sh
# Times the original SLK_img2pix_cmd and xd-art-cli on the same images with the same
# preset, and writes the medians to docs/img/compare-original.txt and a bar chart to
# docs/img/compare-original.svg.
#
# usage: scripts/compare-original.sh [path to a SoftLK-tools checkout] [runs]
#
# Without a path the original is cloned into a temporary directory. Its command line
# tool is built without SDL: scripts/original-cmd-shim.c stands in for the three image
# functions it takes from the GUI library. Needs git, gcc with OpenMP, and cargo.
#
# Every run is a whole process: start, load, convert, save, exit. Both programs read
# the same JSON preset (xd-art through --recipe) and use all cores. One warm-up run per
# case is not counted.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
runs=${2:-10}
cc=${CC:-gcc}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

if [ $# -ge 1 ] && [ -n "$1" ]; then
    src=$(cd "$1" && pwd)
else
    git clone --quiet --depth 1 https://github.com/Captain4LK/SoftLK-tools "$tmp/SoftLK-tools"
    src="$tmp/SoftLK-tools"
fi

# The original command line tool.
mkdir -p "$tmp/stub"
cat >"$tmp/stub/HLH_gui.h" <<'EOF'
#include <stdint.h>
#include <stdio.h>
uint32_t *HLH_gui_image_load(FILE *fp, int *width, int *height);
void HLH_gui_image_free(uint32_t *pix);
void HLH_gui_image_save(FILE *fp, uint32_t *data, int width, int height, const char *ext);
EOF
s="$src/SLK_img2pixel"
lua=""
for f in "$src"/external/lua/*.c; do
    case "$f" in */lua.c | */luac.c | */onelua.c) continue ;; esac
    lua="$lua $f"
done
orig="$tmp/img2pix_cmd"
# shellcheck disable=SC2086
(cd "$s" && $cc -std=c99 -O3 -fopenmp -w -include stdbool.h \
    -I"$tmp/stub" -I"$src/HLH_gui" -I"$src/3rd" -I"$src" -I"$s" \
    main/main_cmd.c tint.c sharp.c sample.c postprocess.c palette.c kmeans.c hscb.c \
    gamma.c dither.c blur.c \
    "$src/shared/rand.c" "$src/shared/png.c" "$src/shared/pcx.c" "$src/shared/image.c" \
    "$src/shared/color.c" "$src/shared/gif.c" "$src/shared/gif_read.c" \
    "$src/HLH/HLH_json.c" "$src/HLH/HLH_string.c" "$root/scripts/original-cmd-shim.c" \
    $lua -lm -o "$orig")
commit=$(cd "$src" && git rev-parse --short HEAD 2>/dev/null || echo unknown)

# xd-art and the test images.
(cd "$root" && cargo build -q --release -p xd-art-cli &&
    cargo build -q --release -p xd-art-bench --example make_image)
xd="$root/target/release/xd-art-cli"
make="$root/target/release/examples/make_image"
"$make" "$tmp/tiny.png" 16 16
"$make" "$tmp/hd.png" 1920 1080
"$make" "$tmp/vga.png" 640 480

# Presets: the original's defaults (64x64, nearest, bayer 4x4, redmean, its 256 color
# palette) with the size and dither mode changed per case.
"$orig" --dump-defaults >"$tmp/default.json"
preset() { # name width height dither_mode
    sed -e "s/\"size_absolute_x\":[0-9]*/\"size_absolute_x\":$2/" \
        -e "s/\"size_absolute_y\":[0-9]*/\"size_absolute_y\":$3/" \
        -e "s/\"dither_dither_mode\":[0-9]*/\"dither_dither_mode\":$4/" \
        "$tmp/default.json" >"$tmp/$1.json"
}
preset startup 64 64 2
preset bayer 160 90 2
preset floyd 320 180 6
preset vga 640 480 6

now() { date +%s%N; }
median() {
    sort -n | awk '{ v[NR] = $1 } END {
        m = (NR % 2) ? v[(NR + 1) / 2] : (v[NR / 2] + v[NR / 2 + 1]) / 2
        printf "%.1f", m / 1000000
    }'
}
time_cmd() { # runs a command $runs times after one warm-up, prints the median in ms
    "$@" >/dev/null 2>&1
    i=0
    : >"$tmp/t"
    while [ "$i" -lt "$runs" ]; do
        a=$(now)
        "$@" >/dev/null 2>&1
        b=$(now)
        echo $((b - a)) >>"$tmp/t"
        i=$((i + 1))
    done
    median <"$tmp/t"
}

out="$root/docs/img"
mkdir -p "$out"
txt="$out/compare-original.txt"
{
    echo "# scripts/compare-original.sh, $runs runs per case, median ms of the whole process"
    echo "# original: SoftLK-tools $commit, built with $($cc --version | head -n 1)"
    echo "# xd-art: $(git -C "$root" rev-parse --short HEAD), release profile"
    echo "# system: $(uname -srm), $(getconf _NPROCESSORS_ONLN 2>/dev/null || echo '?') threads"
    echo "# case original xd-art"
} >"$txt"
for c in "startup:tiny:16x16 to 64x64, bayer 4x4" \
    "bayer:hd:1920x1080 to 160x90, bayer 4x4" \
    "floyd:hd:1920x1080 to 320x180, Floyd-Steinberg" \
    "vga:vga:640x480 to 640x480, Floyd-Steinberg"; do
    name=${c%%:*}
    rest=${c#*:}
    img=${rest%%:*}
    o=$(time_cmd "$orig" --in "$tmp/$img.png" --out "$tmp/o.png" --preset "$tmp/$name.json")
    x=$(time_cmd "$xd" convert "$tmp/$img.png" -o "$tmp/x.png" -f -q --recipe "$tmp/$name.json")
    echo "$name $o $x" >>"$txt"
    printf '%-8s original %8s ms   xd-art %8s ms   (%s)\n' "$name" "$o" "$x" "${rest#*:}"
done

# Bar chart: two bars per case, each labeled with its time.
awk '
    /^#/ { next }
    { n++; name[n] = $1; o[n] = $2; x[n] = $3; if ($2 > max) max = $2; if ($3 > max) max = $3 }
    END {
        w = 640; rowh = 54; top = 40; left = 80; bar = 220
        h = top + n * rowh + 30
        printf "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"%d\" height=\"%d\" font-family=\"sans-serif\" font-size=\"12\">\n", w, h
        printf "<rect width=\"100%%\" height=\"100%%\" fill=\"#16181d\"/>\n"
        printf "<text x=\"16\" y=\"24\" fill=\"#e6e6e6\" font-size=\"14\">whole process time, median (ms), lower is better</text>\n"
        for (i = 1; i <= n; i++) {
            y = top + (i - 1) * rowh
            printf "<text x=\"16\" y=\"%d\" fill=\"#b8b8b8\">%s</text>\n", y + 26, name[i]
            ow = o[i] / max * (w - left - 120); xw = x[i] / max * (w - left - 120)
            printf "<rect x=\"%d\" y=\"%d\" width=\"%.1f\" height=\"18\" fill=\"#8a8f98\"/>\n", left, y + 6, ow
            printf "<text x=\"%.1f\" y=\"%d\" fill=\"#e6e6e6\">%s original</text>\n", left + ow + 6, y + 19, o[i]
            printf "<rect x=\"%d\" y=\"%d\" width=\"%.1f\" height=\"18\" fill=\"#9b87f5\"/>\n", left, y + 27, xw
            printf "<text x=\"%.1f\" y=\"%d\" fill=\"#e6e6e6\">%s xd-art</text>\n", left + xw + 6, y + 40, x[i]
        }
        printf "</svg>\n"
    }' "$txt" >"$out/compare-original.svg"
echo "wrote $txt and $out/compare-original.svg"
