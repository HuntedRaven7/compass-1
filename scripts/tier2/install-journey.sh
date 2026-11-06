#!/usr/bin/env bash
# The Flatpak install journey against real Mutter and the real
# xdg-desktop-portal (#314): fresh install, first run from the application
# entry, start at login through the Background portal, logout and login,
# upgrade while running, uninstall, reinstall and `compass data remove`.
#
# Run BY `headless-gnome-spike.sh`, which brings the compositor and a session
# bus up first. Needs `xdg-desktop-portal` and `xdg-desktop-portal-gnome`,
# which the bus starts on demand.
#
# WHAT STANDS IN FOR THE SESSION, AND WHAT DOES NOT
#
# GNOME Shell and gnome-session do not run in this container (see
# `headless-gnome-spike.sh`), so the script does what they would do, with the
# files the install and the portal left behind:
#
#   - "open Compass from the application grid" runs the exported desktop
#     entry's Exec line;
#   - "log in" runs the Exec line of the autostart entry the portal wrote in
#     ~/.config/autostart, as gnome-session would;
#   - "log out" is the session ending its processes, with SIGTERM.
#
# The Background portal's dialog is answered in advance through the
# permission store (`flatpak permission-set background ...`), which is where
# GNOME records the person's answer. Its control is the refusal: with the
# answer "no", turning login start on must fail and leave no autostart entry.
# The dialog itself, the shell's app grid and a real logout are the manual
# steps in docs/getting-started.md, "Supported desktops and packages".
set -euo pipefail

readonly APP_ID=org.tunaos.compass
readonly BUNDLE_DIR="${BUNDLE_DIR:-/tmp/bundle}"
readonly WAIT_S="${READY_TIMEOUT_S:-60}"
readonly WORK="${JOURNEY_WORK:-/tmp/tier2-journey}"
readonly LOG="$WORK/session.log"
SENTINEL="journey-$$-$(date +%s)"
readonly SENTINEL
readonly APP_DATA="$HOME/.var/app/$APP_ID"
readonly AUTOSTART="${XDG_CONFIG_HOME:-$HOME/.config}/autostart/$APP_ID.desktop"
# Inside the sandbox each XDG base is under ~/.var/app/<id>; the engine's own
# directories are `compass` in each.
readonly SANDBOX_DIRS=("$APP_DATA/config/compass" "$APP_DATA/data/compass" "$APP_DATA/.local/state/compass")

log() { printf '\n=== %s ===\n' "$*"; }
ok() { printf 'ok: %s\n' "$*"; }
fail() {
  printf 'JOURNEY FAILED: %s\n' "$*" >&2
  printf -- '--- compass processes ---\n' >&2
  ps -C compass -o pid=,args= >&2 || true
  if [ -s "$LOG" ]; then
    printf -- '--- %s (last 40 lines) ---\n' "$LOG" >&2
    tail -n 40 "$LOG" >&2
  fi
  exit 1
}

compass() { timeout "$WAIT_S" flatpak --user run "$APP_ID" "$@"; }
in_sandbox() { timeout "$WAIT_S" flatpak --user run --command=sh "$APP_ID" -c "$1"; }

# The pids of this app's Compass processes in one role (`start`, `serve`,
# `launcher-child`), seen from the host. Only processes whose executable is
# `compass` count: the `flatpak run` and `bwrap` around them repeat the
# command line, and a one-off `compass ping` has another role.
role_pids() {
  ps -C compass -o pid=,args= | awk -v role="$1" '{
    for (i = 3; i <= NF; i++) {
      if ($i ~ /^(start|serve|launcher-child|ui)$/) { if ($i == role) print $1; break }
    }
  }'
}
role_count() { role_pids "$1" | grep -c . || true; }

# Exactly one session: one `start` that owns the lease, one engine and one
# window process. Two of anything is the duplicate this journey gates on.
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
  while [ -n "$(ps -C compass -o pid= || true)" ]; do
    [ "$SECONDS" -lt "$deadline" ] || fail "$1: Compass processes outlived it"
    sleep 1
  done
  ok "$1: no Compass process is left"
}
wait_engine() {
  local deadline=$((SECONDS + WAIT_S))
  until compass ping >/dev/null 2>&1; do
    [ "$SECONDS" -lt "$deadline" ] || fail "$1: the engine never answered"
    sleep 1
  done
  # The window process is the last to come up.
  until [ "$(role_count launcher-child)" -ge 1 ]; do
    [ "$SECONDS" -lt "$deadline" ] || fail "$1: no window process came up"
    sleep 1
  done
}
window_open() { compass state open >/dev/null 2>&1; }

# An Exec line from a desktop entry, without its field codes, run detached
# the way the shell or gnome-session would.
exec_line() {
  sed -n 's/^Exec=//p' "$1" | head -1 | sed -E 's/ ?@@[a-z]?//g; s/ ?%[a-zA-Z]//g'
}
launch() {
  local line
  line=$(exec_line "$1")
  [ -n "$line" ] || fail "no Exec line in $1"
  printf '$ %s\n' "$line"
  local argv
  read -ra argv <<<"$line"
  setsid "${argv[@]}" >>"$LOG" 2>&1 </dev/null &
}
app_entry() {
  printf '%s/export/share/applications/%s.desktop' \
    "$(flatpak --user info --show-location "$APP_ID")" "$APP_ID"
}
commit_of_engine() {
  sed -n 's/^app-commit=//p' "/proc/$(engine_pid)/root/.flatpak-info"
}

logout() {
  local pids
  pids=$(ps -C compass -o pid= | tr '\n' ' ')
  [ -n "${pids// /}" ] || fail "log out: nothing was running"
  # shellcheck disable=SC2086 # one pid per word
  kill -TERM $pids
  no_session "$1"
}
shutdown_session() {
  compass shutdown >/dev/null || fail "$1: \`compass shutdown\` failed"
  no_session "$1"
}

write_sentinels() {
  in_sandbox "set -e
    for dir in \"\$XDG_CONFIG_HOME/compass\" \"\$XDG_DATA_HOME/compass\" \"\$XDG_STATE_HOME/compass\"; do
      mkdir -p \"\$dir\" && printf '%s\n' '$SENTINEL' >\"\$dir/journey-sentinel\"
    done" || fail "could not write the sentinels inside the sandbox"
}
sentinels_kept() {
  local dir
  for dir in "${SANDBOX_DIRS[@]}"; do
    [ "$(cat "$dir/journey-sentinel" 2>/dev/null)" = "$SENTINEL" ] ||
      fail "$1: the sentinel in $dir is gone"
  done
  grep -q '"nord"' "$APP_DATA/config/compass/compass.json" ||
    fail "$1: compass.json lost the theme set before it"
  ok "$1: config, data and state sentinels and compass.json are intact"
}

cleanup() {
  local status=$?
  compass shutdown >/dev/null 2>&1 || true
  pkill -TERM -x compass 2>/dev/null || true
  exit "$status"
}
trap cleanup EXIT

[ -n "${WAYLAND_DISPLAY:-}" ] || fail "WAYLAND_DISPLAY is not set; run this under headless-gnome-spike.sh"
bundle="$(find "$BUNDLE_DIR" -name '*.flatpak' -type f | head -1)"
[ -n "$bundle" ] || fail "no .flatpak bundle under $BUNDLE_DIR"
mkdir -p "$WORK"
: >"$LOG"

log "a clean slate: no Compass, no data, no autostart entry"
if flatpak --user info "$APP_ID" >/dev/null 2>&1; then
  flatpak --user uninstall -y --noninteractive --delete-data "$APP_ID"
fi
rm -rf "$APP_DATA" "$AUTOSTART"
flatpak permission-reset "$APP_ID" >/dev/null 2>&1 || true
[ -z "$(ps -C compass -o pid= || true)" ] || fail "Compass is already running"

# The portal picks its backend from XDG_CURRENT_DESKTOP, and the bus starts
# it with the activation environment, not this shell's.
export XDG_CURRENT_DESKTOP=GNOME
dbus-update-activation-environment WAYLAND_DISPLAY XDG_CURRENT_DESKTOP XDG_RUNTIME_DIR
version="$(gdbus call --session -d org.freedesktop.portal.Desktop -o /org/freedesktop/portal/desktop \
  -m org.freedesktop.DBus.Properties.Get org.freedesktop.portal.Background version)" ||
  fail "the session has no Background portal"
ok "Background portal $version"

log "1. fresh install"
flatpak --user install -y --noninteractive --bundle "$bundle" >/dev/null
first_commit=$(flatpak --user info --show-commit "$APP_ID")
entry=$(app_entry)
[ -f "$entry" ] || fail "the install exported no application entry"
grep -q '^NoDisplay=true' "$entry" && fail "the application entry is hidden from the app grid"
ok "installed $first_commit, with a visible application entry"

log "2. first run from the application entry"
launch "$entry"
wait_engine "first run"
window_open || fail "first run: the launcher window is not open"
one_session "first run"
first_engine=$(engine_pid)

log "3. repeated activation reaches the running session"
launch "$entry"
sleep 2
one_session "second activation"
[ "$(engine_pid)" = "$first_engine" ] || fail "second activation replaced the engine"
window_open || fail "second activation: the window is not open"
compass hide >/dev/null || fail "the window did not hide"
window_open && fail "the window still reports open after Hide"
ok "the same engine answered and the window hid"

compass theme set nord >/dev/null || fail "could not set a theme"
write_sentinels
sentinels_kept "after the first run"

log "4. start at login: the refusal first, as the control"
flatpak permission-set background background "$APP_ID" no
if compass login-start enable --json >"$WORK/refused.json"; then
  fail "login start reported success although the desktop refused"
fi
grep -q '"state": "refused"' "$WORK/refused.json" || fail "a refusal was not reported: $(cat "$WORK/refused.json")"
[ ! -e "$AUTOSTART" ] || fail "a refused request still left $AUTOSTART"
ok "refused: no autostart entry, state refused"

log "4. start at login, allowed"
flatpak permission-set background background "$APP_ID" yes
compass login-start enable --json >"$WORK/enabled.json" ||
  fail "login start did not turn on: $(cat "$WORK/enabled.json")"
grep -q '"mechanism": "portal"' "$WORK/enabled.json" || fail "not through the portal: $(cat "$WORK/enabled.json")"
[ -f "$AUTOSTART" ] || fail "the portal wrote no autostart entry"
grep -q "^Exec=.*$APP_ID start --hidden$" "$AUTOSTART" || fail "the autostart entry does not start Compass hidden: $(cat "$AUTOSTART")"
ok "the portal wrote $AUTOSTART"

log "5. log out and log in"
logout "log out with the window resident"
launch "$AUTOSTART"
wait_engine "login"
one_session "login"
window_open && fail "login: start --hidden opened a window"
ok "login: resident and hidden"

log "5. login again while running, then the app grid"
launch "$AUTOSTART"
sleep 2
one_session "a second autostart"
login_engine=$(engine_pid)
launch "$entry"
deadline=$((SECONDS + WAIT_S))
until window_open; do
  [ "$SECONDS" -lt "$deadline" ] || fail "the app grid did not show the resident window"
  sleep 1
done
one_session "app grid after login"
[ "$(engine_pid)" = "$login_engine" ] || fail "the app grid replaced the engine"
compass hide >/dev/null

log "5. a clean shutdown, then log in again"
shutdown_session "compass shutdown"
launch "$AUTOSTART"
wait_engine "login after shutdown"
one_session "login after shutdown"
window_open && fail "login after shutdown: a window opened"

log "6. upgrade while running"
# The rebuilt bundle: the installed build's files and metadata, exported
# again with one file added, as a new commit on the same ref. It is the shape
# of a real update (another commit replaces the deployed one) and the added
# file tells the two deployments apart from inside the sandbox.
rm -rf "$WORK/build" "$WORK/repo"
mkdir -p "$WORK/build"
location=$(flatpak --user info --show-location "$APP_ID")
cp -a "$location/files" "$location/metadata" "$WORK/build/"
printf '%s\n' "$SENTINEL" >"$WORK/build/files/share/compass/journey-build"
flatpak build-finish "$WORK/build" >/dev/null
flatpak build-export --subject="tier 2 install journey: the rebuilt bundle" \
  "$WORK/repo" "$WORK/build" >/dev/null
flatpak build-bundle "$WORK/repo" "$WORK/upgrade.flatpak" "$APP_ID"
running_engine=$(engine_pid)
flatpak --user install -y --noninteractive --bundle "$WORK/upgrade.flatpak" >/dev/null
second_commit=$(flatpak --user info --show-commit "$APP_ID")
[ "$second_commit" != "$first_commit" ] || fail "the rebuilt bundle did not install a new commit"
ok "upgraded $first_commit -> $second_commit"
one_session "during the upgrade"
[ "$(engine_pid)" = "$running_engine" ] || fail "the upgrade replaced the running engine"
[ "$(commit_of_engine)" = "$first_commit" ] || fail "the running engine is not the old commit"
ok "Flatpak left the running engine on the old commit, as documented"
sentinels_kept "after the upgrade"

shutdown_session "shutdown after the upgrade"
launch "$AUTOSTART"
wait_engine "login after the upgrade"
one_session "login after the upgrade"
[ "$(commit_of_engine)" = "$second_commit" ] || fail "after the restart the engine is not the new commit"
[ "$(cat "/proc/$(engine_pid)/root/app/share/compass/journey-build" 2>/dev/null)" = "$SENTINEL" ] ||
  fail "the restarted engine does not see the rebuilt bundle's files"
ok "the next login runs $second_commit"
sentinels_kept "after the upgraded engine started"
compass login-start status --json | grep -q '"mechanism": "portal"' ||
  fail "the upgrade lost login start"

log "7. turn login start off, then uninstall"
compass login-start disable --json >"$WORK/disabled.json" ||
  fail "login start did not turn off: $(cat "$WORK/disabled.json")"
[ ! -e "$AUTOSTART" ] || fail "turning login start off left $AUTOSTART"
ok "the portal removed the autostart entry"
shutdown_session "shutdown before uninstalling"
flatpak --user uninstall -y --noninteractive "$APP_ID" >/dev/null
flatpak --user info "$APP_ID" >/dev/null 2>&1 && fail "still installed"
[ -z "$(ps -C compass -o pid= || true)" ] || fail "Compass runs after uninstalling"
sentinels_kept "after uninstalling"

log "8. reinstall picks the data up; compass data remove deletes it"
flatpak --user install -y --noninteractive --bundle "$bundle" >/dev/null
sentinels_kept "after reinstalling"
compass data remove >"$WORK/dry-run.txt" || fail "the dry run failed"
for dir in "${SANDBOX_DIRS[@]}"; do
  [ -f "$dir/journey-sentinel" ] || fail "the dry run deleted $dir"
done
compass data remove --yes >"$WORK/removed.txt" || fail "data remove --yes failed: $(cat "$WORK/removed.txt")"
for dir in "${SANDBOX_DIRS[@]}"; do
  [ ! -e "$dir" ] || fail "data remove --yes left $dir"
done
ok "the dry run kept everything, --yes removed config, data and state"
flatpak --user uninstall -y --noninteractive "$APP_ID" >/dev/null

log "JOURNEY PASSED"
echo "Fresh install, first run, repeated activation, login start through the portal"
echo "(and its refusal), logout and login, upgrade while running, uninstall,"
echo "reinstall and data removal, with exactly one session throughout."
