#!/bin/sh
# Regenerates crates/xd-art-core/tests/golden/ from the original SLK_img2pixel sources.
#
# usage: scripts/gen-goldens.sh [path to a SoftLK-tools checkout]
#
# Without an argument the original is cloned into a temporary directory. Only the
# algorithm files are compiled (no SDL, no GUI), together with scripts/golden-harness.c.
# Needs git, gcc with OpenMP and the math library.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
out="$root/crates/xd-art-core/tests/golden"
cc=${CC:-gcc}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

if [ $# -ge 1 ]; then
    src=$(cd "$1" && pwd)
else
    git clone --quiet --depth 1 https://github.com/Captain4LK/SoftLK-tools "$tmp/SoftLK-tools"
    src="$tmp/SoftLK-tools"
fi

# color.c includes the GUI header for a few shared types; an empty one is enough here.
mkdir -p "$tmp/stub"
: > "$tmp/stub/HLH_gui.h"
# HLH.h expects the C library headers to be included before its implementation.
printf '#include <stdio.h>\n#include <stdlib.h>\n#include <string.h>\n' > "$tmp/hlh.c"
printf '#define HLH_IMPLEMENTATION\n#include "HLH.h"\n' >> "$tmp/hlh.c"

s="$src/SLK_img2pixel"
$cc -std=c99 -O2 -fopenmp -w \
    -I"$tmp/stub" -I"$src/HLH_gui" -I"$src/3rd" -I"$src" -I"$s" \
    "$root/scripts/golden-harness.c" \
    "$s/blur.c" "$s/sample.c" "$s/sharp.c" "$s/hscb.c" "$s/gamma.c" "$s/tint.c" \
    "$s/dither.c" "$s/kmeans.c" "$src/shared/color.c" "$src/shared/rand.c" "$tmp/hlh.c" \
    -lm -o "$tmp/harness"

mkdir -p "$out"
rm -f "$out"/*.rgba
"$tmp/harness" "$out"
(cd "$src" && git rev-parse HEAD 2>/dev/null || echo unknown) > "$out/SOURCE"
echo "wrote $(ls "$out"/*.rgba | wc -l) files to $out"
