#!/usr/bin/env bash
# Builds the Compass (Rust engine) AppImage from release binaries.
#
#   cargo build --release --locked -p compass -p compass-sandbox -p compass-browser-host -p compass-input-server --bins
#   scripts/build-extension-runtime.sh          # optional; see REQUIRE_RUNTIME
#   packaging/appimage/build-appimage.sh
#
# Standard tooling, pinned by version and checksum: linuxdeploy bundles the
# shared libraries the binaries link (libcrypto, for SQLCipher) and leaves the
# graphics stack to the host, which is what wgpu and winit dlopen anyway;
# appimagetool packs the result. The AppDir itself is laid out by
# scripts/packaging/install-rust-engine.sh, the same install every other output
# uses, plus Node -- which, as in the Flatpak, ships inside.
#
# Node is copied in AFTER linuxdeploy on purpose. linuxdeploy would bundle its
# libstdc++ into usr/lib, and the extension sandbox (Landlock) lets Node read
# its own directory and the system's libraries but not the AppImage's usr/lib,
# so a sandboxed Node could not load it. Official Node binaries are built
# against an old glibc and need nothing a desktop lacks.
#
# Environment:
#   BIN_DIR      release binaries (default target/release)
#   OUT          output file (default Compass-<version>-x86_64.AppImage)
#   TOOLS_DIR    download cache (default .build/appimage-tools)
#   REQUIRE_RUNTIME=1  fail when the extension runtime bundle is missing
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
arch="$(uname -m)"
if [ "$arch" != x86_64 ]; then
  echo "only x86_64 is pinned below; got $arch" >&2
  exit 1
fi

LINUXDEPLOY_VERSION=1-alpha-20251107-1
LINUXDEPLOY_SHA256=c20cd71e3a4e3b80c3483cef793cda3f4e990aca14014d23c544ca3ce1270b4d
APPIMAGETOOL_VERSION=1.9.1
APPIMAGETOOL_SHA256=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
# The Flatpak ships node22 from the SDK extension; this is the same line.
NODE_VERSION=v22.23.3
NODE_SHA256=df450af89261115ef9f9e3830c3eeb2cc9213b63c720b1af623cb5dcbe2e02de

bin_dir="${BIN_DIR:-$repo_root/target/release}"
tools="${TOOLS_DIR:-$repo_root/.build/appimage-tools}"
mkdir -p "$tools"

fetch() {
  local url="$1" sha="$2" out="$3"
  if [ ! -f "$out" ] || ! echo "$sha  $out" | sha256sum -c - >/dev/null 2>&1; then
    echo "fetching $url"
    curl -sSfL "$url" -o "$out.part"
    echo "$sha  $out.part" | sha256sum -c - >/dev/null
    mv "$out.part" "$out"
  fi
}

fetch "https://github.com/linuxdeploy/linuxdeploy/releases/download/$LINUXDEPLOY_VERSION/linuxdeploy-x86_64.AppImage" \
  "$LINUXDEPLOY_SHA256" "$tools/linuxdeploy-x86_64.AppImage"
fetch "https://github.com/AppImage/appimagetool/releases/download/$APPIMAGETOOL_VERSION/appimagetool-x86_64.AppImage" \
  "$APPIMAGETOOL_SHA256" "$tools/appimagetool-x86_64.AppImage"
fetch "https://nodejs.org/dist/$NODE_VERSION/node-$NODE_VERSION-linux-x64.tar.xz" \
  "$NODE_SHA256" "$tools/node-$NODE_VERSION-linux-x64.tar.xz"
chmod +x "$tools"/*.AppImage

# The tools are AppImages themselves; CI containers have no FUSE to mount them.
export APPIMAGE_EXTRACT_AND_RUN=1

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
appdir="$work/AppDir"

DESTDIR="$appdir" PREFIX=/usr LIBEXECDIR=/usr/libexec BIN_DIR="$bin_dir" \
  "$repo_root/scripts/packaging/install-rust-engine.sh"

app_id=org.tunaos.compass
"$tools/linuxdeploy-x86_64.AppImage" \
  --appdir "$appdir" \
  --executable "$appdir/usr/bin/compass" \
  --deploy-deps-only "$appdir/usr/libexec/compass/compass-file-indexer" \
  --deploy-deps-only "$appdir/usr/libexec/compass/compass-sandbox-exec" \
  --desktop-file "$appdir/usr/share/applications/$app_id.desktop" \
  --icon-file "$appdir/usr/share/icons/hicolor/scalable/apps/$app_id.svg" \
  --custom-apprun "$repo_root/packaging/appimage/AppRun"

tar -xJf "$tools/node-$NODE_VERSION-linux-x64.tar.xz" -C "$work" \
  "node-$NODE_VERSION-linux-x64/bin/node"
install -Dm755 "$work/node-$NODE_VERSION-linux-x64/bin/node" "$appdir/usr/bin/node"

version="$("$appdir/usr/bin/compass" --version | awk '{print $2}')"
out="${OUT:-$repo_root/Compass-$version-x86_64.AppImage}"

# --no-appstream: the metainfo is validated where it is reviewed, by the
# Flathub tooling; appimagetool's check needs appstreamcli, which the build
# containers do not all have, and would make the same file pass in one place
# and fail in another.
ARCH=x86_64 VERSION="$version" "$tools/appimagetool-x86_64.AppImage" \
  --no-appstream "$appdir" "$out"
echo "built $out"
