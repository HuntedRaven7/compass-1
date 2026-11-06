#!/usr/bin/env bash
#
# The Orca gate (#305): Orca reads and operates the launcher on real Mutter.
#
# Runs under `headless-gnome-spike.sh`, which brings up the session bus and
# the compositor. This turns accessibility on the way GNOME's screen-reader
# switch does, starts Orca with its debug log as the record of what it said,
# starts the launcher hidden, and hands over to `orca-probe.py`, which
# summons it, types, arrows, opens the action panel, presses an action, fills
# in a form and hides it again, checking the AT-SPI tree and Orca's speech
# after each step.
#
# The launcher is the installed Flatpak (`APP_ID`, the default, which is what
# CI runs: the bundle the pull request built) or a binary (`COMPASS_BIN`, for
# a local build).
#
# THE CONTROL, AND WHY IT IS NOT OPTIONAL
#
# A gate that has only ever been green is indistinguishable from one that
# tests nothing (prove-smoke.sh says how often this repository found that).
# So it runs twice against the same compositor and Orca: once as a user's
# launcher runs (must pass), and once as a launcher Orca cannot read (must
# fail). For the Flatpak that is the sandbox without `org.a11y.Bus`, which is
# how the bundle shipped until this gate's first run found it; for a binary,
# an accessibility bus address that does not exist. If the probe passes
# either, it is not reading the launcher.
#
# What it cannot see: GNOME Shell (it does not run here, see the spike's
# header) and a speech synthesiser. Orca logs each utterance before asking
# speech-dispatcher to say it, so the log is what a person would have heard;
# the voice itself is the manual checklist's
# (docs/rust-engine/ORCA-CHECKLIST.md).

set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
readonly HERE
readonly ORCA_LOG="${ORCA_LOG:-/tmp/orca-debug.log}"
readonly ORCA_OUT="${ORCA_OUT:-/tmp/orca.out}"
readonly LAUNCHER_LOG="${LAUNCHER_LOG:-/tmp/tier2-orca-launcher.log}"
readonly TRANSCRIPT="${TRANSCRIPT:-/tmp/orca-transcript.md}"
readonly READY_TIMEOUT_S="${READY_TIMEOUT_S:-60}"
readonly APP_ID="${APP_ID:-org.tunaos.compass}"
readonly COMPASS_BIN="${COMPASS_BIN:-}"

log() { printf '\n=== %s ===\n' "$*"; }
fail() {
  printf 'ORCA GATE FAILED: %s\n' "$*" >&2
  exit 1
}

[ -n "${WAYLAND_DISPLAY:-}" ] ||
  fail "WAYLAND_DISPLAY is not set: run this under headless-gnome-spike.sh"
for tool in orca gdbus python3; do
  command -v "$tool" >/dev/null || fail "$tool is not installed"
done
python3 -c 'import gi; gi.require_version("Atspi", "2.0")' ||
  fail "python3-gobject cannot load Atspi-2.0 (at-spi2-core's typelib)"

socket="${XDG_RUNTIME_DIR:?}/tier2-orca.sock"

# compass ARGS...: the launcher's command line, against this gate's socket.
compass() {
  if [ -n "$COMPASS_BIN" ]; then
    "$COMPASS_BIN" --socket "$socket" "$@"
  else
    flatpak --user run "$APP_ID" --socket "$socket" "$@"
  fi
}

launcher_pid=
orca_pid=
stop_launcher() {
  if [ -n "$launcher_pid" ]; then
    compass shutdown >/dev/null 2>&1 || true
    kill "$launcher_pid" 2>/dev/null || true
    wait "$launcher_pid" 2>/dev/null || true
    launcher_pid=
  fi
  # `flatpak run` returning does not stop the sandbox (tier2.yaml, finding
  # 4), and a stale engine would answer the next run.
  if [ -z "$COMPASS_BIN" ]; then
    flatpak kill "$APP_ID" 2>/dev/null || true
  fi
}
cleanup() {
  stop_launcher
  if [ -n "$orca_pid" ]; then
    kill "$orca_pid" 2>/dev/null || true
    wait "$orca_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT

a11y_set() {
  gdbus call --session --dest org.a11y.Bus --object-path /org/a11y/bus \
    --method org.freedesktop.DBus.Properties.Set org.a11y.Status "$1" "<$2>" >/dev/null
}

log "accessibility on, as Settings' screen-reader switch turns it on"
# Activates at-spi-bus-launcher on the session bus, which starts the
# accessibility bus and the registry.
a11y_set IsEnabled true
# Newer at-spi2-core (Fedora 45) no longer has ScreenReaderEnabled; Orca
# itself does not need it, so its absence is reported, not fatal.
a11y_set ScreenReaderEnabled true ||
  echo "note: org.a11y.Status has no ScreenReaderEnabled here"
gdbus call --session --dest org.a11y.Bus --object-path /org/a11y/bus \
  --method org.a11y.Bus.GetAddress

log "starting $(orca --version 2>/dev/null | tail -1)"
# Orca's own --debug-file is block-buffered: what it said last sits in the
# buffer until more is said or it exits, and a probe tailing the log would
# wait on an utterance that was made. So Orca is started with its debug
# output on a line-buffered file instead -- the same messages, at the same
# level --debug sets, written as they are made.
# `exec`, so the background job is Orca itself and cleanup's kill reaches it.
run_orca() {
  exec python3 - "$ORCA_LOG" "$(command -v orca)" <<'PY'
import runpy
import sys

from orca import debug

log, orca = sys.argv[1], sys.argv[2]
debug.debugLevel = debug.LEVEL_ALL
debug.debugFile = open(log, "w", encoding="utf-8", buffering=1)
sys.argv = ["orca", "--replace"]
runpy.run_path(orca, run_name="__main__")
PY
}
: >"$ORCA_LOG"
run_orca >"$ORCA_OUT" 2>&1 &
orca_pid=$!
deadline=$((SECONDS + READY_TIMEOUT_S))
until grep -q "SPEECH OUTPUT: 'Screen reader on.'" "$ORCA_LOG" 2>/dev/null; do
  kill -0 "$orca_pid" 2>/dev/null || {
    cat "$ORCA_OUT" >&2
    fail "Orca exited during startup"
  }
  [ "$SECONDS" -lt "$deadline" ] || fail "Orca never said it was on"
  sleep 1
done
echo "ok: Orca is reading the session"

# start_launcher readable|unreadable: the launcher, hidden.
#
# COMPASS_NO_ONBOARDING: the first-run flow is a view of its own, and this
# gate is about the launcher a returning user summons.
start_launcher() {
  local env=(COMPASS_NO_ONBOARDING=1 RUST_LOG=warn)
  if [ -n "$COMPASS_BIN" ]; then
    if [ "$1" = unreadable ]; then
      env+=(AT_SPI_BUS_ADDRESS=unix:path=/nonexistent/at-spi-bus)
    fi
    env "${env[@]}" "$COMPASS_BIN" --socket "$socket" start --hidden \
      >>"$LAUNCHER_LOG" 2>&1 &
  else
    local run=()
    if [ "$1" = unreadable ]; then
      run+=(--no-talk-name=org.a11y.Bus)
    fi
    flatpak --user run "${env[@]/#/--env=}" "${run[@]}" "$APP_ID" \
      --socket "$socket" start --hidden >>"$LAUNCHER_LOG" 2>&1 &
  fi
  launcher_pid=$!
  local deadline=$((SECONDS + READY_TIMEOUT_S))
  until compass ping >/dev/null 2>&1; do
    kill -0 "$launcher_pid" 2>/dev/null || {
      cat "$LAUNCHER_LOG" >&2
      fail "the launcher exited during startup"
    }
    [ "$SECONDS" -lt "$deadline" ] || fail "the launcher never answered ping"
    sleep 1
  done
}

probe() {
  local summon
  if [ -n "$COMPASS_BIN" ]; then
    summon=("$COMPASS_BIN" --socket "$socket" show)
  else
    summon=(flatpak --user run "$APP_ID" --socket "$socket" show)
  fi
  python3 "$HERE/orca-probe.py" --orca-log "$ORCA_LOG" "$@" -- "${summon[@]}"
}

: >"$LAUNCHER_LOG"
log "the launcher, as a screen-reader user runs it (must pass)"
start_launcher readable
status=0
probe --transcript "$TRANSCRIPT" || status=$?
stop_launcher
if [ "$status" -ne 0 ]; then
  echo "--- the launcher's log ---" >&2
  tail -n 50 "$LAUNCHER_LOG" >&2
  fail "Orca could not read or operate the launcher"
fi

log "control: a launcher Orca cannot read (must fail)"
start_launcher unreadable
status=0
probe || status=$?
stop_launcher
if [ "$status" -eq 0 ]; then
  fail "the probe passed a launcher that is not on the accessibility bus, so it is not reading the launcher"
fi
echo "ok: the control failed, as it must"

log "ORCA GATE PASSED"
echo "Orca read and operated the launcher on real Mutter: the search field, the"
echo "typed query, the result count, the selection, the action panel, an action"
echo "pressed through AT-SPI, and a form's text field and checkbox."
echo "Transcript: $TRANSCRIPT"
