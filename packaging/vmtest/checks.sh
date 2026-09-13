#!/usr/bin/env bash
# The VM tier's assertions about compass, run inside the guest.
#
# These are `corral vmtest --check` commands, which arrive over SSH as root, one
# fresh shell each. They live in a script baked into the test image rather than
# as shell one-liners in the workflow for three reasons: `bash -n` can check
# them before CI does, quoting them through YAML and ssh twice is how subtle
# bugs get in, and the shape of "wait for X, then assert Y" does not survive
# being folded onto one line.
#
# Each subcommand is one assertion, so a failing run names which one broke
# instead of reporting that a compound command exited 1. corral runs every check
# even after one fails, so the report is the whole picture.
set -euo pipefail

APP=com.vicinae.Vicinae
INSTALLATION=compass
SESSION_USER=compass
REPORT=/tmp/compass-doctor.json

# uid of the autologin user. Everything about a session is addressed by it.
uid() { id -u "$SESSION_USER"; }

# wait_for runs a command until it succeeds, up to a timeout. Used instead of a
# fixed sleep because llvmpipe makes GNOME's timing wildly variable — ADR-0010
# says to key assertions off states, never off durations.
wait_for() {
  local what="$1" seconds="$2"; shift 2
  local deadline=$((SECONDS + seconds))
  until "$@"; do
    if (( SECONDS >= deadline )); then
      echo "timed out after ${seconds}s waiting for: $what" >&2
      return 1
    fi
    sleep 2
  done
  echo "$what: after ${SECONDS}s"
}

# The session's Wayland socket name. Read from the runtime directory rather than
# assumed to be wayland-0: it is whatever the compositor bound, and guessing
# would make session.type fail for a reason that has nothing to do with us.
wayland_display() {
  local dir
  dir="/run/user/$(uid)"
  find "$dir" -maxdepth 1 -name 'wayland-[0-9]*' -printf '%f\n' 2>/dev/null | sort | head -1
}

case "${1:?usage: checks.sh <subcommand>}" in

  # 1. The Flatpak is in the image at all. If this fails, the layering in
  #    Containerfile.compass is wrong and nothing below can mean anything.
  installed)
    flatpak --installation="$INSTALLATION" list --app --columns=application,version,branch
    flatpak --installation="$INSTALLATION" list --app --columns=application | grep -qx "$APP"
    ;;

  # 2. GDM autologin reached a real user session. A greeter would still paint,
  #    still answer SSH and still pass --require-paint, so this is the assertion
  #    that separates "the OS booted" from "a user is logged in" — and every
  #    check below is meaningless without it.
  session)
    wait_for "a session for $SESSION_USER" 180 \
      bash -c 'loginctl list-sessions --no-legend | grep -qw compass'
    wait_for "the user session bus" 120 test -S "/run/user/$(uid)/bus"
    wait_for "a wayland socket" 120 bash -c '[ -n "$(find "/run/user/$(id -u compass)" -maxdepth 1 -name "wayland-[0-9]*" -print -quit 2>/dev/null)" ]'
    loginctl list-sessions --no-legend
    loginctl show-session "$(loginctl list-sessions --no-legend | awk '$3=="compass"{print $1; exit}')" \
      -p Type -p State -p Active -p Remote || true
    ;;

  # 3. Run the diagnostic from inside the session, as the user, with the session's
  #    own environment — which is the entire point of this tier. The same binary
  #    runs in the Flatpak CI job, but there it runs in a container with no
  #    desktop, so half its checks cannot say anything true.
  #
  #    This subcommand does not assert; it produces the evidence. `doctor` without
  #    --check-only always exits zero by design, so a pass here means only that
  #    the sandboxed binary started in a real session. That is worth knowing on
  #    its own, and the assertions are the next subcommand.
  doctor)
    u="$(uid)"
    runuser -u "$SESSION_USER" -- env \
      XDG_RUNTIME_DIR="/run/user/$u" \
      DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$u/bus" \
      WAYLAND_DISPLAY="$(wayland_display)" \
      XDG_SESSION_TYPE=wayland \
      flatpak run --installation="$INSTALLATION" "$APP" \
        --socket /tmp/compass-vmtest.sock doctor --json > "$REPORT"
    cat "$REPORT"
    ;;

  # 4. The claims worth gating on.
  #
  #    Only three, and each is a fact about the target platform that no other
  #    tier can establish:
  #      session.type    — a Wayland session exists and the sandbox can see it
  #      dbus.session    — the session bus is reachable from inside the sandbox
  #      portal.desktop  — xdg-desktop-portal answers there
  #    Everything else is printed and gated on nothing. In particular
  #    portal.global-shortcuts and gnome.shell-extension are evidence only: the
  #    first is Spike A's subject and not yet expected to work, and this
  #    repository ships no Shell extension for the second to find (ADR-0004).
  doctor-assert)
    REPORT="$REPORT" python3 - <<'PY'
import json, os, sys

report = json.load(open(os.environ['REPORT']))
checks = report.get('checks') or []
if not checks:
    sys.exit('doctor produced no checks')

status = {c['name']: c['status'] for c in checks}
for c in checks:
    print(f"  {c['status']:5} {c['name']}: {c.get('detail', '')}")

required = ['session.type', 'dbus.session', 'portal.desktop']
bad = [f"{n}={status.get(n, 'MISSING')}" for n in required if status.get(n) != 'ok']
if bad:
    sys.exit('not ok in a real GNOME session: ' + ', '.join(bad))
print('\nall gated checks ok:', ', '.join(required))
PY
    ;;

  *)
    echo "unknown subcommand: $1" >&2
    exit 64
    ;;
esac
