#!/usr/bin/env bash
# Harvest a desktop-entry corpus from a real system into the test corpus.
#
# The corpus committed to this repo is seeded from a CI container, which has almost no
# applications installed. Run this on a real desktop -- ideally a Bluefin box, since that is the
# first target and its RPM + Flatpak + Homebrew mix is what users actually have -- and commit the
# result. See docs/rust-engine/PLAN.md section 8.1.
#
#   ./scripts/harvest-desktop-corpus.sh [--limit N] [--out DIR]
#
# Files are named <source>--<original-stem>.desktop so entries from different roots cannot
# collide, and existing files are left alone so re-running is safe.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$REPO_ROOT/crates/compass-testkit/corpus/desktop-entries/real"
LIMIT=0

while [ $# -gt 0 ]; do
  case "$1" in
    --limit) LIMIT="$2"; shift 2 ;;
    --out) OUT="$2"; shift 2 ;;
    -h|--help) sed -n '2,14p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

mkdir -p "$OUT"

# Label each root so the provenance survives into the filename.
declare -a ROOTS=(
  "host:/usr/share/applications"
  "hostlocal:/usr/local/share/applications"
  "user:$HOME/.local/share/applications"
  "flatpak:/var/lib/flatpak/exports/share/applications"
  "flatpakuser:$HOME/.local/share/flatpak/exports/share/applications"
  "snap:/var/lib/snapd/desktop/applications"
  "brew:/home/linuxbrew/.linuxbrew/share/applications"
)

copied=0
skipped=0

for spec in "${ROOTS[@]}"; do
  label="${spec%%:*}"
  dir="${spec#*:}"
  [ -d "$dir" ] || continue

  while IFS= read -r -d '' file; do
    if [ "$LIMIT" -gt 0 ] && [ "$copied" -ge "$LIMIT" ]; then break 2; fi

    dest="$OUT/${label}--$(basename "$file")"
    if [ -e "$dest" ]; then
      skipped=$((skipped + 1))
      continue
    fi

    # Copy bytes verbatim: encoding and line endings are part of what we test.
    cp --no-preserve=mode,ownership "$file" "$dest"
    copied=$((copied + 1))
  done < <(find "$dir" -maxdepth 1 -type f -name '*.desktop' -print0 | sort -z)
done

total=$(find "$OUT" -type f -name '*.desktop' | wc -l)

echo "harvested $copied new, skipped $skipped existing"
echo "corpus now holds $total real entries in $OUT"

if [ "$total" -lt 500 ]; then
  echo
  echo "note: the plan targets ~500 real entries; this machine yielded $total." >&2
  echo "run this on additional machines, or install more Flatpaks, and commit the union." >&2
fi
