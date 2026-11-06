#!/usr/bin/env bash
# The Arch package's install journey, through real pacman transactions (#314):
# install, a user's engine running, upgrade, removal, reinstall and
# `compass data remove`.
#
# What it proves that `crates/compass/tests/packaging_lifecycle.rs` cannot:
# that pacman runs `arch/compass.install`'s hooks at the right moments, that
# `post_upgrade` finds a user's running engine and leaves it running on the
# replaced binary, and that `pre_remove` stops it through its own socket as
# that user, so it exits by itself rather than being signalled.
#
# The container has no systemd user manager, so each user here is a user
# running Compass "some other way" (packaging/README.md): the message on
# upgrade, `compass shutdown` as the user on removal. The `systemctl --user`
# branch stays covered by the stubbed tests and by hand. No display either,
# so the user's Compass is the engine (`compass serve`); the window's
# single-instance lease is the tier-2 and wlroots journeys'.
#
# The upgrade is the same package installed again, which pacman runs as an
# upgrade (pre_upgrade/post_upgrade) and which replaces /usr/bin/compass with
# a new file, as a version bump does.
#
# Usage, as root: scripts/packaging/arch-journey.sh path/to/compass-git-*.pkg.tar.zst
set -euo pipefail

readonly PKG=${1:?usage: arch-journey.sh path/to/compass-git-*.pkg.tar.zst}
readonly NAME=compass-git
readonly BIN=/usr/bin/compass
readonly WHO=${JOURNEY_USER:-journey}
readonly WAIT_S=${WAIT_S:-30}
SENTINEL="journey-$$-$(date +%s)"
readonly SENTINEL

log() { printf '\n=== %s ===\n' "$*"; }
ok() { printf 'ok: %s\n' "$*"; }
fail() {
  printf 'JOURNEY FAILED: %s\n' "$*" >&2
  pgrep -a compass >&2 || true
  exit 1
}

[ "$(id -u)" = 0 ] || fail "run this as root: it installs and removes a package"
[ -f "$PKG" ] || fail "no package at $PKG"

id -u "$WHO" >/dev/null 2>&1 || useradd -m "$WHO"
uid=$(id -u "$WHO")
home=$(getent passwd "$WHO" | cut -d: -f6)
readonly uid home
readonly RUNTIME="/run/user/$uid"
readonly DIRS=("$home/.config/compass" "$home/.local/share/compass" "$home/.local/state/compass")
mkdir -p "$RUNTIME"
chown "$WHO:" "$RUNTIME"
chmod 700 "$RUNTIME"

as_user() {
  runuser -u "$WHO" -- env -i HOME="$home" PATH=/usr/bin:/bin XDG_RUNTIME_DIR="$RUNTIME" \
    COMPASS_DISABLE_AUTO_RATE_REFRESH=1 COMPASS_UPDATE_FEED_URL=http://127.0.0.1:9/releases/latest \
    "$@"
}
engines() { pgrep -u "$uid" -x compass || true; }
start_engine() {
  as_user setsid "$BIN" serve --no-hotkey >>/tmp/arch-journey-engine.log 2>&1 </dev/null &
  local deadline=$((SECONDS + WAIT_S))
  until as_user "$BIN" ping >/dev/null 2>&1; do
    [ "$SECONDS" -lt "$deadline" ] || fail "the user's engine never answered"
    sleep 0.5
  done
}
one_engine() {
  local count
  count=$(engines | grep -c . || true)
  [ "$count" = 1 ] || fail "$1: expected exactly one engine for $WHO, found $count"
  ok "$1: one engine for $WHO"
}
sentinels_kept() {
  local dir
  for dir in "${DIRS[@]}"; do
    [ "$(cat "$dir/journey-sentinel" 2>/dev/null)" = "$SENTINEL" ] ||
      fail "$1: the sentinel in $dir is gone"
  done
  grep -q '"nord"' "$home/.config/compass/compass.json" ||
    fail "$1: compass.json lost the theme set before it"
  ok "$1: config, data and state sentinels and compass.json are intact"
}
pacman_says() { # pacman_says <log> <text>: the hook's message reached the person
  grep -qF "$2" "$1" || { cat "$1" >&2; fail "pacman did not print: $2"; }
}

cleanup() {
  local status=$?
  for pid in $(engines); do kill "$pid" 2>/dev/null || true; done
  exit "$status"
}
trap cleanup EXIT

log "1. fresh install"
if pacman -Q "$NAME" >/dev/null 2>&1; then
  pacman -R --noconfirm "$NAME" >/dev/null
fi
[ -z "$(engines)" ] || fail "$WHO already runs Compass"
pacman -U --noconfirm "$PKG" >/tmp/arch-journey-install.log 2>&1 ||
  { cat /tmp/arch-journey-install.log >&2; fail "pacman -U failed"; }
[ -x "$BIN" ] || fail "$BIN was not installed"
ok "installed $(pacman -Q "$NAME")"

log "2. the user runs Compass"
start_engine
one_engine "first run"
engine=$(engines)
# The hooks find users through /proc/<pid>/exe, which root may read for
# another user's process only with CAP_SYS_PTRACE. A real system's root has
# it; a container may not, and then every hook would quietly find no one.
readlink "/proc/$engine/exe" >/dev/null ||
  fail "root cannot read /proc/$engine/exe here; run the container with --cap-add=SYS_PTRACE"
as_user "$BIN" theme set nord >/dev/null || fail "could not set a theme"
for dir in "${DIRS[@]}"; do
  as_user mkdir -p "$dir"
  printf '%s\n' "$SENTINEL" | as_user tee "$dir/journey-sentinel" >/dev/null
done
sentinels_kept "after the first run"

log "3. upgrade while it runs"
pacman -U --noconfirm "$PKG" >/tmp/arch-journey-upgrade.log 2>&1 ||
  { cat /tmp/arch-journey-upgrade.log >&2; fail "the upgrade failed"; }
pacman_says /tmp/arch-journey-upgrade.log "Compass is still running the previous version for $WHO."
one_engine "after the upgrade"
[ "$(engines)" = "$engine" ] || fail "the upgrade replaced the running engine"
[ "$(readlink "/proc/$engine/exe")" = "$BIN (deleted)" ] ||
  fail "the binary was not replaced under the running engine"
as_user "$BIN" ping >/dev/null || fail "the old engine stopped answering"
ok "post_upgrade named $WHO and left the engine running on the old binary"
sentinels_kept "after the upgrade"

log "4. restart onto the new version"
as_user "$BIN" shutdown >/dev/null || fail "compass shutdown failed"
deadline=$((SECONDS + WAIT_S))
while [ -n "$(engines)" ]; do
  [ "$SECONDS" -lt "$deadline" ] || fail "the engine outlived compass shutdown"
  sleep 0.5
done
start_engine
one_engine "after the restart"
engine=$(engines)
[ "$(readlink "/proc/$engine/exe")" = "$BIN" ] || fail "the restart is not the new binary"

log "5. remove while it runs"
pacman -R --noconfirm "$NAME" >/tmp/arch-journey-remove.log 2>&1 ||
  { cat /tmp/arch-journey-remove.log >&2; fail "pacman -R failed"; }
grep -qF "could not be stopped" /tmp/arch-journey-remove.log &&
  { cat /tmp/arch-journey-remove.log >&2; fail "pre_remove could not stop the engine"; }
pacman_says /tmp/arch-journey-remove.log "Each user's Compass settings, extensions and history were kept"
deadline=$((SECONDS + WAIT_S))
while kill -0 "$engine" 2>/dev/null; do
  [ "$SECONDS" -lt "$deadline" ] || fail "pre_remove left the engine running"
  sleep 0.5
done
[ ! -e "$BIN" ] || fail "$BIN is still there"
ok "pre_remove stopped $WHO's engine through its socket"
sentinels_kept "after removal"

log "6. reinstall picks the data up; compass data remove deletes it"
pacman -U --noconfirm "$PKG" >/dev/null 2>&1 || fail "the reinstall failed"
sentinels_kept "after reinstalling"
as_user "$BIN" data remove >/dev/null || fail "the dry run failed"
sentinels_kept "after the dry run"
as_user "$BIN" data remove --yes >/tmp/arch-journey-data.log ||
  { cat /tmp/arch-journey-data.log >&2; fail "data remove --yes failed"; }
for dir in "${DIRS[@]}"; do
  [ ! -e "$dir" ] || fail "data remove --yes left $dir"
done
ok "--yes removed config, data and state"

echo
echo "PASS: install, upgrade and removal through pacman with a user's engine running;"
echo "the hooks handed over, stopped it through its socket and kept its data"
