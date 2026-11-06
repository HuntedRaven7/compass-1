#!/usr/bin/env bash
# Compass against upstream Vicinae, on this machine. See docs/rust-engine/BENCHMARKS.md.
#
# Usage: scripts/bench/compare.sh [OUTPUT_DIR] [RUNS]
#
# Needs: cargo (the pinned toolchain), a C++23 compiler, python3, sway, grim,
# wtype, dbus-daemon, curl, binutils. No GPU, no running session: it starts its
# own headless Sway and D-Bus bus and uses throwaway HOME/XDG directories.
#
# Upstream is the pinned, unmodified v0.29.0 AppImage from scripts/bench/baseline.json,
# verified by SHA-256 before use. It is extracted, not mounted, so FUSE is not needed.
set -euo pipefail

root=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
out=${1:-$root/target/compare/run-$(date -u +%Y%m%dT%H%M%SZ)}
runs=${2:-5}
cache=${COMPASS_BENCH_CACHE:-$root/target/bench-cache}
mkdir -p "$out" "$cache"

baseline() { python3 -c "import json,sys; print(json.load(open('$root/scripts/bench/baseline.json'))[sys.argv[1]])" "$1"; }
tag=$(baseline tag)
url=$(baseline url)
want=$(baseline sha256)
appimage=$cache/Vicinae-$tag-x86_64.AppImage

if [ ! -f "$appimage" ]; then
  curl -fL --retry 3 -o "$appimage.part" "$url"
  mv "$appimage.part" "$appimage"
fi
got=$(sha256sum "$appimage" | cut -d' ' -f1)
[ "$got" = "$want" ] || { echo "upstream AppImage checksum mismatch: $got != $want" >&2; exit 1; }
chmod +x "$appimage"
rm -rf "$out/upstream"
mkdir -p "$out/upstream"
(cd "$out/upstream" && "$appimage" --appimage-extract >/dev/null)

# The AppImage bundles Qt but takes GL dispatch, font and X client libraries from the
# host. Name the missing ones here, not as an exit 127 in the middle of the first run.
server="$out/upstream/squashfs-root/usr/bin/vicinae-server"
missing=$(LD_LIBRARY_PATH="$out/upstream/squashfs-root/usr/lib" ldd "$server" | awk '/not found/ {print $1}' | sort -u)
if [ -n "$missing" ]; then
  echo "upstream's vicinae-server needs host libraries this machine lacks: ${missing//$'\n'/ }" >&2
  echo "On Debian or Ubuntu: apt install libopengl0 libglx0 libegl1 libfontconfig1 libfreetype6 libx11-xcb1" >&2
  exit 1
fi

cargo build --release --locked -p compass -p compass-testkit -p compass-sandbox \
  --bin compass --bin compass-file-indexer --bin fuzzy-throughput --bin compass-sandbox-exec \
  --bin head-to-head
target=$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')

c++ -std=c++23 -O2 -I"$root/scripts/bench/upstream/fuzzy/include" -o "$out/cpp-rank" "$root/scripts/bench/fuzzy/cpp_rank.cpp"

{
  echo "commit: $(git -C "$root" rev-parse HEAD)$(git -C "$root" diff --quiet HEAD || echo ' (dirty)')"
  rustc --version
  c++ --version | head -1
  sway --version
} >"$out/versions.txt"

python3 "$root/scripts/bench/compare.py" \
  --compass "$target/release/compass" \
  --upstream "$out/upstream/squashfs-root/AppRun" \
  --upstream-appimage "$appimage" \
  --head-to-head "$target/release/head-to-head" \
  --cpp-rank "$out/cpp-rank" \
  --fuzzy-throughput "$target/release/fuzzy-throughput" \
  --out "$out" --runs "$runs"

echo "report: $out/report.json" >&2
echo "upstream gate points: $out/points.jsonl" >&2
