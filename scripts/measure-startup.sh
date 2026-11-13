#!/bin/sh
# Measures xd-art startup: runs the GUI several times with the trace on, lets it quit by
# itself, and prints the median time of every step.
#
# usage: scripts/measure-startup.sh [runs] [image]
#
#   runs   how many runs (default 15); the first one is a warm-up and not counted
#   image  open this image and wait until it is shown; without it the run ends at the
#          first frame of the empty window
#
# Uses target/release/xd-art (build it first with `cargo build --release -p xd-art-gui`),
# or the binary in $XD_ART. Needs a desktop session: the window really opens.
#
# The step times come from the program itself (XD_ART_TRACE) and count from the start
# of main. The "whole run" time is measured from outside with hyperfine when it is
# installed, otherwise with `date +%s%N` around each run; it includes process creation
# and exit.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
runs=${1:-15}
image=${2:-}
bin=${XD_ART:-"$root/target/release/xd-art"}
[ -x "$bin" ] || [ -x "$bin.exe" ] || {
    echo "no binary at $bin; build with: cargo build --release -p xd-art-gui" >&2
    exit 1
}

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
log="$tmp/trace.log"

if [ -n "$image" ]; then
    export XD_ART_TRACE_EXIT=image
else
    export XD_ART_TRACE_EXIT=frame
fi

# Warm-up: fills the file cache so every counted run starts from the same place.
XD_ART_TRACE="$tmp/warmup.log" "$bin" $image

: >"$tmp/wall.txt"
i=0
while [ "$i" -lt "$runs" ]; do
    a=$(date +%s%N)
    XD_ART_TRACE="$log" "$bin" $image
    b=$(date +%s%N)
    case "$a$b" in
    *N*) ;; # date without %N: no outside timing
    *) echo "$(((b - a) / 1000))" >>"$tmp/wall.txt" ;;
    esac
    i=$((i + 1))
done

median() {
    sort -n | awk '{ v[NR] = $1 } END {
        if (NR == 0) { print "-"; exit }
        m = (NR % 2) ? v[(NR + 1) / 2] : (v[NR / 2] + v[NR / 2 + 1]) / 2
        printf "%.2f", m
    }'
}

echo "runs: $runs, binary: $bin${image:+, image: $image}"
for step in platform config window "first frame" decoded "image shown"; do
    values=$(grep "trace: $step " "$log" | awk '{ print $(NF - 1) }')
    [ -n "$values" ] || continue
    printf '%-14s %9s ms (median)\n' "$step" "$(echo "$values" | median)"
done
if [ -s "$tmp/wall.txt" ]; then
    printf '%-14s %9s ms (median, from outside)\n' "whole run" \
        "$(awk '{ print $1 / 1000 }' "$tmp/wall.txt" | median)"
fi

if command -v hyperfine >/dev/null 2>&1; then
    echo
    hyperfine --warmup 1 --runs "$runs" "$bin $image"
fi
