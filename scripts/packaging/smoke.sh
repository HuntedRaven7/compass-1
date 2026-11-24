#!/usr/bin/env bash
# Suite 5 install-matrix smoke (PLAN.md section 8.6): one script, every output.
#
# Proves an installed Rust engine is complete and starts, with no desktop:
#
#   1. layout  -- with --prefix, every file the Flatpak ships is where the
#                 engine looks for it (scripts/packaging/install-rust-engine.sh
#                 is the list);
#   2. version -- `compass --version` runs;
#   3. schema  -- `compass config schema` is the published schema;
#   4. doctor  -- `doctor --json` produces a report, and `doctor --check-only`
#                 FAILS here, because a CI container has no display, bus or
#                 portal: a doctor that passed would be the useless one 8.6
#                 warns about;
#   5. engine  -- `serve` starts on a private socket, `ping` answers, doctor
#                 now reports the socket ok, and `shutdown` stops it.
#
# Usage:
#   scripts/packaging/smoke.sh [--prefix DIR] [--libexecdir DIR]
#                              [--keep-env] [--socket PATH] -- COMMAND...
#
#   COMMAND is how to run `compass`: /usr/bin/compass, result/bin/compass,
#   ./Compass-x86_64.AppImage (set APPIMAGE_EXTRACT_AND_RUN=1 where there is no
#   FUSE), or `flatpak run org.tunaos.compass`. --libexecdir defaults to
#   PREFIX/libexec. REQUIRE_RUNTIME=1 also requires the extension runtime
#   bundle.
#
#   --keep-env skips the private home and runtime directory, for a COMMAND
#   that needs the real ones (flatpak run does); --socket then names a socket
#   path every invocation of COMMAND can see.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
prefix=""
libexecdir=""
keep_env=0
socket=""

while [ $# -gt 0 ]; do
  case "$1" in
    --prefix) prefix="$2"; shift 2 ;;
    --libexecdir) libexecdir="$2"; shift 2 ;;
    --keep-env) keep_env=1; shift ;;
    --socket) socket="$2"; shift 2 ;;
    --) shift; break ;;
    *) echo "unknown argument $1" >&2; exit 2 ;;
  esac
done
if [ $# -eq 0 ]; then
  echo "usage: $0 [--prefix DIR] [--libexecdir DIR] -- COMMAND..." >&2
  exit 2
fi
compass=("$@")

failures=0
pass() { echo "ok   $*"; }
fail() { echo "::error::$*"; failures=$((failures + 1)); }

# --- 1. layout ---------------------------------------------------------------
if [ -n "$prefix" ]; then
  libexecdir="${libexecdir:-$prefix/libexec}"
  expect_file() {
    if [ -s "$1" ]; then pass "$1"; else fail "missing $1"; fi
  }
  expect_exec() {
    if [ -x "$1" ]; then pass "$1"; else fail "missing or not executable: $1"; fi
  }
  expect_exec "$prefix/bin/compass"
  expect_exec "$libexecdir/compass/compass-sandbox-exec"
  expect_exec "$libexecdir/compass/compass-file-indexer"
  expect_exec "$libexecdir/compass/compass-browser-host"
  expect_file "$prefix/share/applications/org.tunaos.compass.desktop"
  expect_file "$prefix/share/metainfo/org.tunaos.compass.metainfo.xml"
  expect_file "$prefix/share/icons/hicolor/scalable/apps/org.tunaos.compass.svg"
  expect_file "$prefix/share/compass/builtin-icons/question-mark-circle.svg"
  expect_file "$prefix/lib/systemd/user/compass.service"
  expect_file "$prefix/share/compass/compass.schema.json"
  for script in web-search unit-converter epoch-converter generators quick-notes; do
    expect_file "$prefix/share/compass/scripts/$script/script.toml"
    expect_file "$prefix/share/compass/scripts/$script/main.rhai"
  done
  if [ "${REQUIRE_RUNTIME:-0}" = 1 ]; then
    expect_file "$prefix/share/compass/extension-runtime.js"
  fi
  if command -v desktop-file-validate >/dev/null 2>&1; then
    if desktop-file-validate "$prefix/share/applications/org.tunaos.compass.desktop"; then
      pass "desktop-file-validate"
    else
      fail "the desktop file does not validate"
    fi
  fi
fi

# Everything below runs in a private, empty session: no display, no bus, a
# throwaway home. Nothing reaches the runner's own configuration or socket.
work="$(mktemp -d)"
engine_pid=""
cleanup() {
  if [ -n "$engine_pid" ] && kill -0 "$engine_pid" 2>/dev/null; then
    kill "$engine_pid" 2>/dev/null || true
  fi
  rm -rf "$work"
}
trap cleanup EXIT

if [ "$keep_env" = 0 ]; then
  mkdir -p "$work/home" "$work/runtime"
  chmod 700 "$work/runtime"
  export HOME="$work/home"
  export XDG_CONFIG_HOME="$work/home/.config"
  export XDG_DATA_HOME="$work/home/.local/share"
  export XDG_CACHE_HOME="$work/home/.cache"
  export XDG_RUNTIME_DIR="$work/runtime"
  export DBUS_SESSION_BUS_ADDRESS="unix:path=$work/no-bus"
fi
unset WAYLAND_DISPLAY DISPLAY
socket="${socket:-$work/ipc.sock}"
rm -f "$socket"

# --- 2. version --------------------------------------------------------------
if version="$("${compass[@]}" --version)" && [[ "$version" == compass\ * ]]; then
  pass "--version: $version"
else
  fail "--version did not print a version: ${version:-<nothing>}"
fi

# --- 3. schema ---------------------------------------------------------------
if "${compass[@]}" config schema > "$work/schema.json" \
  && cmp -s "$work/schema.json" "$repo_root/packaging/schema/compass.schema.json"; then
  pass "config schema matches packaging/schema/compass.schema.json"
else
  fail "config schema differs from the published schema"
fi

# --- 4. doctor ---------------------------------------------------------------
doctor_status() {
  python3 - "$1" "$2" <<'PY'
import json, sys
report = json.load(open(sys.argv[1]))
checks = {c["name"]: c["status"] for c in report["checks"]}
print(checks.get(sys.argv[2], "absent"))
PY
}

if "${compass[@]}" --socket "$socket" doctor --json > "$work/doctor.json" \
  && [ "$(python3 -c 'import json,sys; print(len(json.load(open(sys.argv[1]))["checks"]))' "$work/doctor.json")" -gt 0 ]; then
  pass "doctor --json produced $(python3 -c 'import json,sys; print(len(json.load(open(sys.argv[1]))["checks"]))' "$work/doctor.json") checks"
else
  fail "doctor --json did not produce a report"
fi

if "${compass[@]}" --socket "$socket" doctor --check-only > "$work/check-only.txt"; then
  fail "doctor --check-only passed with no display, bus or portal; it must detect absence"
else
  pass "doctor --check-only detects the missing session (exit $?)"
fi
# With the real environment there may be a bus, so only the private session
# can promise which checks must fail.
if [ "$keep_env" = 0 ]; then
  if [ "$(doctor_status "$work/doctor.json" portal.desktop)" = fail ]; then
    pass "doctor reports portal.desktop absent"
  else
    fail "doctor did not report the missing portal"
  fi
fi

# --- 5. engine ---------------------------------------------------------------
"${compass[@]}" --socket "$socket" serve --no-hotkey > "$work/engine.log" 2>&1 &
engine_pid=$!

# Bounded by time rather than attempts: one attempt through `flatpak run` or
# an AppImage mount costs far more than one against a plain binary.
answered=0
deadline=$((SECONDS + 60))
while [ "$SECONDS" -lt "$deadline" ]; do
  if "${compass[@]}" --socket "$socket" ping > "$work/ping.txt" 2>&1; then
    answered=1
    break
  fi
  if ! kill -0 "$engine_pid" 2>/dev/null; then
    break
  fi
  sleep 0.1
done
if [ "$answered" = 1 ]; then
  pass "ping: $(cat "$work/ping.txt")"
  "${compass[@]}" --socket "$socket" doctor --json > "$work/doctor-live.json" || true
  if [ "$(doctor_status "$work/doctor-live.json" ipc.socket)" = ok ]; then
    pass "doctor sees the running engine"
  else
    fail "doctor does not see the running engine on $socket"
  fi
  if "${compass[@]}" --socket "$socket" shutdown > /dev/null && wait "$engine_pid"; then
    pass "shutdown stopped the engine"
  else
    fail "shutdown did not stop the engine cleanly"
  fi
  engine_pid=""
else
  fail "the engine never answered ping; its log:"
  sed 's/^/    /' "$work/engine.log"
fi

if [ "$failures" -gt 0 ]; then
  echo "$failures smoke check(s) failed"
  exit 1
fi
echo "all smoke checks passed"
