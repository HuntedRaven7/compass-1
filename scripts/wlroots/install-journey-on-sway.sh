#!/usr/bin/env bash
# The install journey on the non-portal path (#314): an unsandboxed Compass on
# headless Sway with no Background portal, the wlroots row of
# docs/getting-started.md, "Supported desktops and packages".
#
# "Installing" copies the binary into a private prefix on PATH, as a package
# would put it in /usr/bin, and "upgrading" replaces it by rename, as pacman
# does. With no session bus there is no portal, so `compass login-start
# enable` writes Compass's own ~/.config/autostart/compass.desktop, and "log
# in" runs that entry's Exec line, as an XDG autostart implementation would.
# "Log out" is the session ending its processes with SIGTERM.
#
# Gates: exactly one `start`, one engine and one window process after every
# activation and login; `start --hidden` opens no window; the running engine
# survives the binary being replaced and the next login runs the new one;
# config, data and state survive the upgrade and the uninstall;
# `compass data remove --yes` deletes them.
#
# Usage: scripts/wlroots/install-journey-on-sway.sh [path/to/compass]
# Needs: sway, pgrep, setsid.
set -euo pipefail

readonly SOURCE=${1:-target/debug/compass}
readonly WAIT_S=${WAIT_S:-60}

for tool in sway pgrep setsid; do
  command -v "$tool" >/dev/null || { echo "missing: $tool" >&2; exit 2; }
done
[ -x "$SOURCE" ] || { echo "no compass binary at $SOURCE (cargo build -p compass)" >&2; exit 2; }

work=${COMPASS_WLROOTS_WORK:-$(mktemp -d /tmp/compass-journey.XXXXXX)}
mkdir -p "$work"
work=$(cd "$work" && pwd)
readonly work
readonly LOG="$work/journey-session.log"
readonly BIN="$work/prefix/bin/compass"
SENTINEL="journey-$$-$(date +%s)"
readonly SENTINEL
sway_pid=

log() { printf '\n=== %s ===\n' "$*"; }
ok() { printf 'ok: %s\n' "$*"; }
fail() {
  printf 'JOURNEY FAILED: %s\n' "$*" >&2
  pgrep -a compass >&2 || true
  [ -s "$LOG" ] && tail -n 40 "$LOG" >&2
  exit 1
}

mkdir -p "$work/run" "$work/home" "$work/prefix/bin" "$work/empty"
chmod 700 "$work/run"
export HOME="$work/home" XDG_RUNTIME_DIR="$work/run" XDG_CURRENT_DESKTOP=sway \
  XDG_CONFIG_HOME="$work/home/.config" XDG_DATA_HOME="$work/home/.local/share" \
  XDG_CACHE_HOME="$work/home/.cache" XDG_STATE_HOME="$work/home/.local/state" \
  XDG_DATA_DIRS="$work/empty" PATH="$work/prefix/bin:$PATH" \
  DBUS_SESSION_BUS_ADDRESS="unix:path=$work/no-session-bus" \
  COMPASS_DISABLE_AUTO_RATE_REFRESH=1 COMPASS_UPDATE_FEED_URL=http://127.0.0.1:9/releases/latest
unset WAYLAND_DISPLAY DISPLAY SWAYSOCK
readonly AUTOSTART="$XDG_CONFIG_HOME/autostart/compass.desktop"
readonly DIRS=("$XDG_CONFIG_HOME/compass" "$XDG_DATA_HOME/compass" "$XDG_STATE_HOME/compass")

# This journey's processes: those whose executable is the installed binary,
# or was, before an upgrade replaced it.
our_pids() {
  local pid exe
  for pid in $(pgrep -x compass || true); do
    exe=$(readlink "/proc/$pid/exe" 2>/dev/null) || continue
    case "$exe" in "$BIN" | "$BIN (deleted)") echo "$pid" ;; esac
  done
}
role_pids() {
  local pid
  for pid in $(our_pids); do
    tr '\0' '\n' <"/proc/$pid/cmdline" 2>/dev/null |
      awk -v role="$1" -v pid="$pid" '
        NR > 1 && /^(start|serve|launcher-child|ui)$/ { if ($0 == role) print pid; exit }'
  done
}
role_count() { role_pids "$1" | grep -c . || true; }
one_session() {
  local role count
  for role in start serve launcher-child; do
    count=$(role_count "$role")
    [ "$count" = 1 ] || fail "$1: expected exactly one \`compass $role\`, found $count"
  done
  [ "$(role_count ui)" = 0 ] || fail "$1: a stray \`compass ui\` is running"
  ok "$1: one start, one engine, one window process"
}
engine_pid() { role_pids serve; }
no_session() {
  local deadline=$((SECONDS + WAIT_S))
  while [ -n "$(our_pids)" ]; do
    [ "$SECONDS" -lt "$deadline" ] || fail "$1: Compass processes outlived it"
    sleep 0.5
  done
  ok "$1: no Compass process is left"
}
wait_session() {
  local deadline=$((SECONDS + WAIT_S))
  until compass ping >/dev/null 2>&1 && [ "$(role_count launcher-child)" -ge 1 ]; do
    [ "$SECONDS" -lt "$deadline" ] || fail "$1: the session never came up"
    sleep 0.5
  done
}
window_open() { compass state open >/dev/null 2>&1; }
wait_open() {
  local deadline=$((SECONDS + WAIT_S))
  until window_open; do
    [ "$SECONDS" -lt "$deadline" ] || fail "$1: the window never opened"
    sleep 0.5
  done
}
launch() { # launch <command...>, detached as a desktop would start it
  printf '$ %s\n' "$*"
  setsid "$@" >>"$LOG" 2>&1 </dev/null &
}
login() {
  local line argv
  line=$(sed -n 's/^Exec=//p' "$AUTOSTART" | head -1)
  [ -n "$line" ] || fail "no Exec line in $AUTOSTART"
  read -ra argv <<<"$line"
  launch "${argv[@]}"
}
logout() {
  local pids
  pids=$(our_pids | tr '\n' ' ')
  [ -n "${pids// /}" ] || fail "log out: nothing was running"
  # shellcheck disable=SC2086 # one pid per word
  kill -TERM $pids
  no_session "$1"
}
install_binary() { # as a package manager does: a new file, renamed into place
  cp "$SOURCE" "$BIN.new"
  mv -f "$BIN.new" "$BIN"
}
sentinels_kept() {
  local dir
  for dir in "${DIRS[@]}"; do
    [ "$(cat "$dir/journey-sentinel" 2>/dev/null)" = "$SENTINEL" ] ||
      fail "$1: the sentinel in $dir is gone"
  done
  grep -q '"nord"' "$XDG_CONFIG_HOME/compass/compass.json" ||
    fail "$1: compass.json lost the theme set before it"
  ok "$1: config, data and state sentinels and compass.json are intact"
}

cleanup() {
  local status=$?
  if [ -x "$BIN" ]; then compass shutdown >/dev/null 2>&1 || true; fi
  for pid in $(our_pids); do kill "$pid" 2>/dev/null || true; done
  [ -n "$sway_pid" ] && kill "$sway_pid" 2>/dev/null
  wait 2>/dev/null || true
  if [ -n "${COMPASS_WLROOTS_WORK:-}" ]; then echo "logs kept in $work"; else rm -rf "$work"; fi
  exit "$status"
}
trap cleanup EXIT

printf 'output HEADLESS-1 resolution 1280x800 bg #202020 solid_color\nxwayland disable\n' \
  >"$work/journey-sway.cfg"
WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=pixman \
  sway -c "$work/journey-sway.cfg" >"$work/journey-sway.log" 2>&1 &
sway_pid=$!
WAYLAND_DISPLAY=
for _ in $(seq 1 200); do
  for socket in "$XDG_RUNTIME_DIR"/wayland-[0-9]; do
    [ -S "$socket" ] && WAYLAND_DISPLAY=${socket##*/}
  done
  [ -n "$WAYLAND_DISPLAY" ] && break
  sleep 0.1
done
[ -n "$WAYLAND_DISPLAY" ] || { cat "$work/journey-sway.log" >&2; fail "sway never listened"; }
export WAYLAND_DISPLAY
: >"$LOG"

log "1. install"
install_binary
[ "$(command -v compass)" = "$BIN" ] || fail "the installed compass is not the one on PATH"
[ -z "$(our_pids)" ] || fail "Compass is already running"

log "2. first run, as the application entry starts it"
launch compass start
wait_session "first run"
wait_open "first run"
one_session "first run"
first_engine=$(engine_pid)

log "3. repeated activation"
compass start || fail "a second \`compass start\` failed"
one_session "second activation"
[ "$(engine_pid)" = "$first_engine" ] || fail "the second activation replaced the engine"
compass hide >/dev/null
window_open && fail "the window still reports open after Hide"
compass theme set nord >/dev/null || fail "could not set a theme"
for dir in "${DIRS[@]}"; do
  mkdir -p "$dir" && printf '%s\n' "$SENTINEL" >"$dir/journey-sentinel"
done
sentinels_kept "after the first run"

log "4. start at login, without a portal"
compass login-start enable --json >"$work/enabled.json" ||
  fail "login start did not turn on: $(cat "$work/enabled.json")"
grep -q '"mechanism": "autostart-file"' "$work/enabled.json" ||
  fail "not through Compass's own autostart entry: $(cat "$work/enabled.json")"
grep -q '^X-Compass-Login-Start=true$' "$AUTOSTART" || fail "no autostart entry of Compass's own"
grep -q '^Exec=compass start --hidden$' "$AUTOSTART" || fail "the entry does not start Compass hidden"
ok "Compass wrote $AUTOSTART"

log "5. log out and log in"
logout "log out with the window resident"
login
wait_session "login"
one_session "login"
window_open && fail "login: start --hidden opened a window"
login_engine=$(engine_pid)
login
sleep 2
one_session "a second autostart"
launch compass start
wait_open "the app grid after login"
one_session "the app grid after login"
[ "$(engine_pid)" = "$login_engine" ] || fail "the app grid replaced the engine"
compass hide >/dev/null

log "6. upgrade while running"
running_engine=$(engine_pid)
install_binary
[ "$(readlink "/proc/$running_engine/exe")" = "$BIN (deleted)" ] ||
  fail "the running engine was not left on the replaced binary"
compass start --hidden || fail "the new binary's start --hidden failed"
one_session "the new binary's start --hidden beside the old session"
[ "$(engine_pid)" = "$running_engine" ] || fail "the upgrade replaced the running engine"
sentinels_kept "after the upgrade"
compass shutdown >/dev/null || fail "compass shutdown failed"
no_session "compass shutdown after the upgrade"
login
wait_session "login after the upgrade"
one_session "login after the upgrade"
[ "$(readlink "/proc/$(engine_pid)/exe")" = "$BIN" ] || fail "the next login does not run the new binary"
ok "the next login runs the new binary"
compass login-start status --json | grep -q '"mechanism": "autostart-file"' ||
  fail "the upgrade lost login start"

log "7. turn login start off, then uninstall"
compass login-start disable --json >/dev/null || fail "login start did not turn off"
[ ! -e "$AUTOSTART" ] || fail "turning login start off left $AUTOSTART"
compass shutdown >/dev/null || fail "compass shutdown failed"
no_session "shutdown before uninstalling"
rm -f "$BIN"
sentinels_kept "after uninstalling"

log "8. reinstall picks the data up; compass data remove deletes it"
install_binary
sentinels_kept "after reinstalling"
compass data remove >/dev/null || fail "the dry run failed"
sentinels_kept "after the dry run"
compass data remove --yes >"$work/removed.txt" || fail "data remove --yes failed: $(cat "$work/removed.txt")"
for dir in "${DIRS[@]}"; do
  [ ! -e "$dir" ] || fail "data remove --yes left $dir"
done
ok "--yes removed config, data and state"
rm -f "$BIN"

echo
echo "PASS: install, first run, repeated activation, login start without a portal,"
echo "logout and login, upgrade while running, uninstall and data removal, with"
echo "exactly one session throughout"
