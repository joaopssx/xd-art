#!/bin/sh
# Makes the pictures in docs/img/alg-*.png that docs/algorithms.md shows. Every one is
# the same synthetic landscape (xd_art_bench::scene, 256x256) run through
# xd-art-cli with the options written next to it below, so anyone can make them again
# and get the same files.
#
# usage: scripts/gen-doc-images.sh
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
out="$root/docs/img"
mkdir -p "$out"
(cd "$root" && cargo build -q --release -p xd-art-cli &&
    cargo build -q --release -p xd-art-bench --example make_image)
cli="$root/target/release/xd-art-cli"
src="$out/alg-source.png"
"$root/target/release/examples/make_image" "$src" 256 256 scene

run() { # name, then options
    name=$1
    shift
    "$cli" convert "$src" -o "$out/alg-$name.png" -f -q "$@"
    echo "alg-$name.png: $*"
}

# Sampling: 64x64, 32 colors taken from the image, no dithering, shown 4x.
for m in nearest bilinear bicubic lanczos box cluster edge; do
    run "sample-$m" -w 64 -h 64 --sample "$m" --extract 32 -s 4
done

# Blur before sampling.
for r in 0 2 6; do
    run "blur-$r" -w 64 -h 64 --blur "$r" --extract 32 -s 4
done

# Dithering: 96x96 to PICO-8, shown 3x.
for d in none bayer2x2 bayer4x4 bayer8x8 cluster8x8 floyd floyd-mono stucki sierra-lite picocad median-cut; do
    run "dither-$d" -w 96 -h 96 -p pico-8 -d "$d" --amount 0.4 -s 3
done

# Color distance: the same 4 colors of the Game Boy palette, no dithering.
for m in rgb redmean ciede2000; do
    run "metric-$m" -w 96 -h 96 -p game-boy --metric "$m" -s 3
done

# Palettes taken from the image: 8 colors with each method.
for m in k-means median-cut balanced; do
    run "extract-$m" -w 96 -h 96 --extract 8 --method "$m" -s 3
done
