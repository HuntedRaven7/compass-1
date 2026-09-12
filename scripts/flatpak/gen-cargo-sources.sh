#!/usr/bin/env bash
# Regenerates packaging/flatpak/cargo-sources.json from Cargo.lock.
#
# Flathub builds have no network, so every crate has to be declared up front with
# its checksum. That list is derived entirely from Cargo.lock, which means it goes
# stale the moment a dependency changes -- and a stale list fails the Flatpak build
# with a missing-crate error that reads like a registry problem rather than a
# "you forgot to regenerate" problem. CI diffs the regenerated file against the
# committed one so the staleness is caught here instead.
#
# Usage: scripts/flatpak/gen-cargo-sources.sh [output-path]
set -euo pipefail

# Pinned rather than tracking master: this script's output is committed, so an
# upstream change to the generator would show up as an unexplained diff in a PR
# that did not touch dependencies.
GENERATOR_REF="${GENERATOR_REF:-de2225a6dee4818c1339b3cdbf29f90c471fcb7e}"
GENERATOR_URL="https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/${GENERATOR_REF}/cargo/flatpak-cargo-generator.py"
# The commit pin already fixes the content; this catches a corrupted or
# substituted download, which is worth one line for a script we pipe into python.
GENERATOR_SHA256="b373c8ab1a05378ec5d8ed0645c7b127bcec7d2f7a1798694fbc627d570d856c"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
out="${1:-$repo_root/packaging/flatpak/cargo-sources.json}"

workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT

echo "fetching the generator ($GENERATOR_REF)"
curl -sSfL "$GENERATOR_URL" -o "$workdir/gen.py"
echo "$GENERATOR_SHA256  $workdir/gen.py" | sha256sum -c - >/dev/null

echo "generating $out from Cargo.lock"
python3 "$workdir/gen.py" "$repo_root/Cargo.lock" -o "$out"

echo "wrote $(python3 -c "import json,sys; print(len(json.load(open(sys.argv[1]))))" "$out") entries"
