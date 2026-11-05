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

APP=org.tunaos.compass
INSTALLATION=compass
SESSION_USER=compass
REPORT=/tmp/compass-doctor.json
SPIKE_OUT=/tmp/compass-spike-a.json
SPIKE_ERR=/tmp/compass-spike-a.err
SPIKE_DONE=/tmp/compass-spike-a.done
UI_ERR=/tmp/compass-ui.err
UI_DONE=/tmp/compass-ui.done
ENGINE_ERR=/tmp/compass-engine.err
ENGINE_DONE=/tmp/compass-engine.done
ENGINE_PIDS=/tmp/compass-engine.pids
CONTROL_ERR=/tmp/compass-control-app.err
CONTROL_DONE=/tmp/compass-control-app.done
KBD_CAP_DIR=/tmp
KBD_CAP=/tmp/compass-kbd-capture.bin
KBD_CAP_DONE=/tmp/compass-kbd-capture.done

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

# True when some process is running the named binary, or when the sentinel says
# it already exited. Compares the resolved /proc/PID/exe rather than matching a
# command line, so it cannot match the shell that is doing the asking.
exe_running() {
  local want="$1" sentinel="$2" exe
  [ -f "$sentinel" ] && return 0
  for d in /proc/[0-9]*; do
    exe="$(readlink "$d/exe" 2>/dev/null || true)"
    case "$exe" in
      */"$want") return 0 ;;
    esac
  done
  return 1
}

# Run a compass CLI command as the session user, inside the Flatpak.
#
# Named `compass_cli`, not `as_session`: `spike-a-evidence` already defines a
# local `as_session` that runs an arbitrary guest binary without the Flatpak,
# and two helpers of the same name doing different things is how the wrong one
# gets called.
#
# Every summon check needs the same seven environment variables, and getting
# XDG_RUNTIME_DIR wrong means talking to a socket that is not the session's --
# which presents as "no engine running" rather than as a mistake here.
#
# THE ARGUMENTS ARE THE CLI'S OWN SUBCOMMAND, with no `compass` in front. The
# Flatpak's entrypoint IS `compass`, so `compass_cli compass ping` runs
# `compass compass ping` -- rejected by the parser, forever. That cost a
# 30-minute VM run, presenting as "the engine never answered a ping" with a
# perfectly healthy engine sitting there. `crates/compass/tests/vmtest_cli.rs`
# now parses these call sites with the real clap definition so the next one
# fails in seconds instead.
compass_cli() {
  local u
  u="$(uid)"
  runuser -u "$SESSION_USER" -- env \
    XDG_RUNTIME_DIR="/run/user/$u" \
    DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$u/bus" \
    WAYLAND_DISPLAY="$(wayland_display)" \
    XDG_SESSION_TYPE=wayland \
    flatpak run --installation="$INSTALLATION" "$APP" "$@"
}

# The launcher's pid, excluding the engine's.
#
# BOTH PROCESSES ARE NAMED `compass`. Before ADR-0015 there was only ever one,
# and `pgrep -u "$SESSION_USER" -x compass | head -1` was unambiguous. Now
# `serve` runs first, so that pgrep matches the engine -- and matching the
# engine is not a cosmetic problem:
#
#   * `launcher-start`'s waiter would be satisfied the instant it began,
#     because a `compass` process already exists, so it would stop waiting for
#     the launcher entirely;
#   * `launcher-diagnose` would report which syscall the *engine* is parked in
#     while claiming to explain the launcher;
#   * `launcher-rss` would measure the wrong process against Phase 1's gate.
#
# So the engine records its pids and everything about the launcher skips them.
# Matching on the command line instead is the obvious alternative and is worse
# here: both are `flatpak run … org.tunaos.compass <verb>` and the inner
# process is a grandchild whose argv is not the one written above.
launcher_pid() {
  local engine="" pid
  [ -f "$ENGINE_PIDS" ] && engine="$(tr '\n' ' ' < "$ENGINE_PIDS")"
  for pid in $(pgrep -u "$SESSION_USER" -x compass 2>/dev/null); do
    case " $engine " in
      *" $pid "*) continue ;;
    esac
    echo "$pid"
    return 0
  done
  return 1
}

# True once the launcher exists, or once it has exited.
launcher_appeared() {
  launcher_pid >/dev/null || [ -f "$UI_DONE" ]
}

# True once the engine answers, or once it has exited.
#
# A shell function rather than a `bash -c` predicate, because `wait_for` calls
# what it is given in the current shell: a subshell would not inherit
# `compass_cli`, `uid` or `APP`, and the check would fail for reasons that have
# nothing to do with the engine.
engine_ready() {
  compass_cli ping >/dev/null 2>&1 || [ -f "$ENGINE_DONE" ]
}

# The session's XDG_CURRENT_DESKTOP, read from gnome-shell's own environment.
#
# Without it doctor cannot tell it is on GNOME and answers
# gnome.shell-extension with "not a GNOME session" -- an ok that proves
# nothing, which is exactly how the gate first passed. Read rather than
# hard-coded, so the gate stays honest if the image's session changes; GNOME
# is the fallback because that is what this image is.
session_desktop() {
  local pid
  pid="$(pgrep -u "$SESSION_USER" -x gnome-shell | head -1)"
  if [ -n "$pid" ]; then
    tr '\0' '\n' < "/proc/$pid/environ" 2>/dev/null \
      | sed -n 's/^XDG_CURRENT_DESKTOP=//p' | head -1 | grep . && return 0
  fi
  echo GNOME
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
      XDG_CURRENT_DESKTOP="$(session_desktop)" \
      flatpak run --installation="$INSTALLATION" "$APP" \
        --socket /tmp/compass-vmtest.sock doctor --json > "$REPORT"
    cat "$REPORT"
    ;;

  # 4. The claims worth gating on.
  #
  #    Each is a fact about the target platform that no other tier can
  #    establish:
  #      session.type          — a Wayland session exists and the sandbox can see it
  #      dbus.session          — the session bus is reachable from inside the sandbox
  #      portal.desktop        — xdg-desktop-portal answers there
  #      gnome.shell-extension — the extension `shell-extension` enabled speaks
  #                              our contract, AS SEEN FROM INSIDE THE SANDBOX:
  #                              the Flatpak's talk-name grant is what this proves,
  #                              and a wrong one looks exactly like "not installed"
  #    Everything else is printed and gated on nothing. In particular
  #    portal.global-shortcuts is evidence only: it is Spike A's subject and not
  #    yet expected to work.
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

required = ['session.type', 'dbus.session', 'portal.desktop', 'gnome.shell-extension']
bad = [f"{n}={status.get(n, 'MISSING')}" for n in required if status.get(n) != 'ok']
if bad:
    sys.exit('not ok in a real GNOME session: ' + ', '.join(bad))

# `ok` alone is not enough for the extension: "not a GNOME session" is also
# ok, and is what an environment doctor misreads looks like. Only "present"
# proves the sandbox reached the extension's objects on org.gnome.Shell.
detail = next(c.get('detail', '') for c in checks if c['name'] == 'gnome.shell-extension')
if not detail.startswith('extension present'):
    sys.exit('gnome.shell-extension is ok without finding the extension: ' + detail)
print('\nall gated checks ok:', ', '.join(required))
PY
    ;;

  # The Compass GNOME Shell extension: enabled the way a user enables one, then
  # asked over the session bus, from outside the sandbox, whether it answers.
  #
  # Two claims, both gated. The Windows object reports our contract version -- so Shell
  # accepted the extension (a shell-version mismatch presents here as "never
  # appeared", which is why `gnome-extensions info` is printed on failure) --
  # and ListWindows returns a well-formed reply. The inside-the-sandbox half is
  # doctor-assert's gnome.shell-extension, which runs after this.
  shell-extension)
    u="$(uid)"
    uuid=compass@tunaos.org
    as_user() {
      runuser -u "$SESSION_USER" -- env \
        XDG_RUNTIME_DIR="/run/user/$u" \
        DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$u/bus" \
        "$@"
    }
    contract_version() {
      as_user gdbus call --session \
        --dest org.gnome.Shell \
        --object-path /org/tunaos/compass/Shell/Windows \
        --method org.freedesktop.DBus.Properties.Get \
        org.tunaos.compass.Shell.Windows Version
    }
    contract_up() { case "$(contract_version 2>/dev/null)" in *"uint32 4>"*) return 0 ;; esac; return 1; }

    if ! as_user gnome-extensions enable "$uuid"; then
      echo "gnome-extensions could not enable $uuid" >&2
      as_user gnome-extensions list --details >&2 2>&1 || true
      exit 1
    fi
    if ! wait_for "the Compass extension to export contract v4" 60 contract_up; then
      echo "Version reads: $(contract_version 2>&1 || true)" >&2
      as_user gnome-extensions info "$uuid" >&2 2>&1 || true
      exit 1
    fi
    echo "extension up: $(contract_version)"

    if ! windows="$(as_user gdbus call --session \
      --dest org.gnome.Shell \
      --object-path /org/tunaos/compass/Shell/Windows \
      --method org.tunaos.compass.Shell.Windows.ListWindows 2>&1)"; then
      echo "ListWindows failed: $windows" >&2
      exit 1
    fi
    echo "ListWindows: $windows"
    # An aa{sv}: gdbus prints `(@aa{sv} [],)` with nothing open (an empty array
    # carries its type) and `([{'id': <uint32 …>, …}],)` otherwise. Anything
    # else is a reply the engine could not decode.
    case "$windows" in
      "(@aa{sv} [],)" | "([{"*"}],)") ;;
      *) echo "ListWindows returned something that is not an array of windows" >&2; exit 1 ;;
    esac

    # Contract 4: GNOME always has at least one workspace, so the reply is a
    # non-empty aa{sv} whose first entry is index 0, and exactly one entry is
    # active. A fresh session is on the first workspace, so switching to
    # index 0 is a no-op the Shell must still accept, answered with `()`.
    if ! workspaces="$(as_user gdbus call --session \
      --dest org.gnome.Shell \
      --object-path /org/tunaos/compass/Shell/Windows \
      --method org.tunaos.compass.Shell.Windows.ListWorkspaces 2>&1)"; then
      echo "ListWorkspaces failed: $workspaces" >&2
      exit 1
    fi
    echo "ListWorkspaces: $workspaces"
    case "$workspaces" in
      "([{'index': <0>"*"}],)") ;;
      *) echo "ListWorkspaces returned something that is not the workspaces from 0" >&2; exit 1 ;;
    esac
    actives="$(grep -o "'active': <true>" <<< "$workspaces" | wc -l || true)"
    if [ "$actives" -ne 1 ]; then
      echo "ListWorkspaces marked $actives workspaces active, not one" >&2
      exit 1
    fi
    if ! switched="$(as_user gdbus call --session \
      --dest org.gnome.Shell \
      --object-path /org/tunaos/compass/Shell/Windows \
      --method org.tunaos.compass.Shell.Windows.ActivateWorkspace 0 2>&1)"; then
      echo "ActivateWorkspace failed: $switched" >&2
      exit 1
    fi
    echo "ActivateWorkspace 0: $switched"
    if [ "$switched" != "()" ]; then
      echo "ActivateWorkspace answered $switched, not ()" >&2
      exit 1
    fi
    ;;

  # Clipboard history end to end (#238): the extension watches the clipboard
  # and the engine records it, read back from the engine's own log.
  #
  # SetClipboard over the session bus is the injection point: it replaces the
  # selection the way a copy presents it, and the extension emits the same
  # change signal the engine records. Two distinct markers separate "recorded
  # at all" from "recorded twice". The assertion reads the recording path's
  # own `clipboard change` decisions rather than `query`: the unified query
  # searches apps, commands, extensions and scripts, and history entries are
  # not in it (they have their own page and request). The engine serves on a
  # private socket with --no-hotkey, so no portal permission prompt can stall
  # the check.
  clipboard-history)
    u="$(uid)"
    # runuser keeps root's HOME, and the keyring daemon resolves its keyring
    # files from HOME: without this every keyring step below operates on
    # /root's keyring while the session daemon keeps the user's locked one.
    user_home="$(getent passwd "$SESSION_USER" | cut -d: -f6)"
    sock=/tmp/compass-clipboard-history.sock
    engine_log=/tmp/compass-clipboard-history-engine.log
    engine_done=/tmp/compass-clipboard-history-engine.done
    marker_a="compass-vmtest-clipboard-alpha"
    marker_b="compass-vmtest-clipboard-beta"
    as_user() {
      runuser -u "$SESSION_USER" -- env \
        HOME="$user_home" \
        XDG_RUNTIME_DIR="/run/user/$u" \
        DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$u/bus" \
        "$@"
    }
    # Exported so `timeout` can run them: timeout execs its argument, and a
    # bare `timeout 120 as_user ...` dies with "No such file or directory".
    # The engine CLI has no socket timeout of its own, so ping and shutdown
    # ride behind timeout too; an engine that never answers must fail the
    # check, not the whole 75-minute job.
    export -f as_user
    export SESSION_USER u user_home INSTALLATION APP sock
    engine() {
      as_user env \
        WAYLAND_DISPLAY="$(wayland_display)" \
        XDG_SESSION_TYPE=wayland \
        flatpak run --installation="$INSTALLATION" "$APP" \
          --socket "$sock" "$@"
    }
    export -f engine wayland_display uid
    # $1: marker text. Gio over python3 carries the exact bytes; spelling a
    # GVariant byte array through gdbus quoting is how subtle bugs get in.
    set_clipboard() {
      as_user python3 - "$1" <<'PY'
import sys
from gi.repository import Gio, GLib
proxy = Gio.DBusProxy.new_sync(
    Gio.bus_get_sync(Gio.BusType.SESSION, None),
    Gio.DBusProxyFlags.NONE, None,
    'org.gnome.Shell',
    '/org/tunaos/compass/Shell/Clipboard',
    'org.tunaos.compass.Shell.Clipboard', None)
proxy.call_sync('SetClipboard',
    # new_tuple, not a '(ay,s)' format string: PyGObject's format parser
    # rejects the byte array there (TypeError on the first VM run).
    # Finite timeout, not -1: a handler that never replies must fail the
    # check, not hang it past the job's timeout.
    GLib.Variant.new_tuple(
        GLib.Variant('ay', sys.argv[1].encode()),
        GLib.Variant('s', 'text/plain')),
    Gio.DBusCallFlags.NONE, 15000, None)
PY
    }
    # Both markers made it through signal, ingest and store: the engine logs
    # one `clipboard change` decision per recorded copy, and two distinct
    # markers Insert rather than bubble up.
    recorded_both() {
      [ "$(grep -c 'clipboard change' "$engine_log" 2>/dev/null)" -ge 2 ] || return 1
      grep -q 'Inserted' "$engine_log" || return 1
    }
    clipboard_version() {
      as_user gdbus call --session \
        --dest org.gnome.Shell \
        --object-path /org/tunaos/compass/Shell/Clipboard \
        --method org.freedesktop.DBus.Properties.Get \
        org.tunaos.compass.Shell.Clipboard Version
    }
    clipboard_up() { case "$(clipboard_version 2>/dev/null)" in *"uint32 4>"*) return 0 ;; esac; return 1; }

    # Self-sufficient like shell-extension: enable the helper the way a user
    # does, then wait for the Clipboard object. Without this the bus has no
    # Clipboard path when this check runs alone. Every step narrates itself
    # and carries a timeout: one round hung fifty minutes silent before its
    # first echo, and silence is not debuggable.
    echo "enabling the Shell extension..."
    if ! timeout 120 bash -c 'as_user gnome-extensions enable compass@tunaos.org'; then
      echo "gnome-extensions could not enable compass@tunaos.org" >&2
      as_user gnome-extensions list --details >&2 2>&1 || true
      exit 1
    fi
    echo "waiting for the Clipboard object..."
    if ! wait_for "the Clipboard object to export contract v4" 60 clipboard_up; then
      echo "Version reads: $(clipboard_version 2>&1 || true)" >&2
      as_user gnome-extensions info compass@tunaos.org >&2 2>&1 || true
      exit 1
    fi
    echo "clipboard up: $(clipboard_version)"

    # An unlocked login keyring, the way a password login leaves one: the
    # engine keeps history's master key there, and without it the service
    # logs "clipboard history unavailable" and records nothing — there is no
    # unencrypted fallback. The roundtrip probe below is the ground truth;
    # the keyring is the VM's throwaway.
    #
    # --unlock is the only non-interactive primitive: it creates a missing
    # login keyring with the password on stdin, but cannot open one whose
    # password it does not know, and a locked collection answers Store with
    # an interaction prompt that hangs headless. A collection over D-Bus is
    # no escape either: the daemon password-protects it through the same
    # prompt. So when the login keyring is unusable, the session daemon is
    # killed and a fresh one is started with a known password.
    keyring_probe() {
      printf %s probe | timeout 60 bash -c 'as_user secret-tool store --label=compass-vmtest compass-vmtest probe' || return 1
      [ "$(timeout 60 bash -c 'as_user secret-tool lookup compass-vmtest probe' </dev/null)" = "probe" ] || return 1
      timeout 60 bash -c 'as_user secret-tool clear compass-vmtest probe' </dev/null >/dev/null || true
      return 0
    }
    secrets_up() {
      as_user gdbus introspect --session \
        --dest org.freedesktop.secrets \
        --object-path /org/freedesktop/secrets 2>/dev/null | grep -q org.freedesktop.Secret.Service
    }
    no_keyring_daemon() {
      # -f with a bracketed first letter: comm caps at 15 chars so -x can
      # never match, and the brackets keep the wrapper's own command line
      # from matching the pattern.
      ! as_user pgrep -f '[g]nome-keyring-daemon' >/dev/null
    }
    if as_user command -v secret-tool >/dev/null; then
      echo "unlocking the login keyring..."
      # An empty password first: the session daemon can hold an
      # auto-created blank login keyring, which our password does not open.
      as_user sh -c 'printf "" | gnome-keyring-daemon --unlock --components=secrets' || true
      as_user sh -c 'printf compass-vmtest-keyring | gnome-keyring-daemon --unlock --components=secrets' || true
      echo "secret service collections:"
      as_user gdbus call --session \
        --dest org.freedesktop.secrets \
        --object-path /org/freedesktop/secrets \
        --method org.freedesktop.DBus.Properties.Get \
        org.freedesktop.Secret.Service Collections 2>&1 || true
      echo "probing the Secret Service roundtrip..."
      if ! keyring_probe; then
        # Polite replacement does not dethrone the session's login daemon:
        # an earlier round showed the original `--daemonize --login`
        # process still owning the bus afterwards with our --replace hung
        # behind it. Kill every keyring daemon outright (including those
        # hung processes), drop the unknown-password login file, and start
        # one fresh daemon with a known password. Mask the user units first
        # so socket activation cannot resurrect one mid-check. Retried, in
        # case the session respawns the old daemon into the gap.
        echo "login keyring not usable; restarting the daemon with a known password..."
        timeout 60 bash -c 'as_user systemctl --user mask gnome-keyring-daemon.service gnome-keyring-daemon.socket' >/dev/null 2>&1 || true
        reset_ok=0
        for attempt in 1 2 3; do
          echo "keyring reset attempt $attempt/3..."
          as_user pkill -f '[g]nome-keyring-daemon' || true
          if ! wait_for "all keyring daemons to exit" 30 no_keyring_daemon; then
            echo "keyring daemons would not exit" >&2
            continue
          fi
          as_user sh -c 'rm -f "$HOME/.local/share/keyrings/login.keyring"' || true
          if ! printf compass-vmtest-keyring | timeout 60 bash -c 'as_user gnome-keyring-daemon --daemonize --unlock --components=secrets'; then
            echo "could not start a fresh keyring daemon" >&2
            continue
          fi
          if ! wait_for "the fresh daemon to answer" 60 secrets_up; then
            echo "the fresh daemon never answered" >&2
            continue
          fi
          if keyring_probe; then reset_ok=1; break; fi
          echo "roundtrip failed on attempt $attempt" >&2
        done
        if [ "$reset_ok" != 1 ]; then
          echo "roundtrip failed on a reset keyring; keyring state:" >&2
          as_user sh -c 'ps -o pid,args -C gnome-keyring-daemon' >&2 2>&1 || true
          timeout 30 bash -c 'as_user busctl --user status org.freedesktop.secrets' >&2 2>&1 || true
          exit 1
        fi
      fi
      echo "secret service roundtrip ok"
    else
      echo "no secret-tool; skipping the keyring roundtrip" >&2
    fi

    # Detached like spike-a-start: ssh waits for the channel otherwise, and
    # this check would never return.
    rm -f "$sock" "$engine_done"
    : > "$engine_log"
    # -vv rides along so the recording decisions land in the log; the
    # assertion below greps them. The CLI's own flags, not flatpak --env:
    # options after the app id reach Compass, not flatpak, which is how a
    # --env ended up as a clap error one round.
    echo "starting the engine on a private socket..."
    setsid bash -c '
      runuser -u "$1" -- env \
        XDG_RUNTIME_DIR="/run/user/$2" \
        DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$2/bus" \
        WAYLAND_DISPLAY="$3" \
        XDG_SESSION_TYPE=wayland \
        flatpak run --installation="$4" \
          "$5" --socket "$6" -vv serve --no-hotkey > "$7" 2>&1
      echo "$?" > "$8"
    ' _ "$SESSION_USER" "$u" "$(wayland_display)" "$INSTALLATION" "$APP" \
      "$sock" "$engine_log" "$engine_done" < /dev/null >> "$engine_log" 2>&1 &

    if ! wait_for "the engine to answer ping" 120 timeout 120 bash -c 'engine ping'; then
      echo "the engine never answered; its log:" >&2
      cat "$engine_log" >&2 || true
      exit 1
    fi
    echo "setting both markers over the bus..."
    set_clipboard "$marker_a" || { echo "SetClipboard failed for marker_a" >&2; timeout 120 bash -c 'engine shutdown' || true; exit 1; }
    set_clipboard "$marker_b" || { echo "SetClipboard failed for marker_b" >&2; timeout 120 bash -c 'engine shutdown' || true; exit 1; }
    if ! wait_for "both markers recorded in history" 60 recorded_both; then
      echo "the markers never landed in history; clipboard lines, then the tail:" >&2
      grep -i clipboard "$engine_log" >&2 | tail -20 || true
      tail -30 "$engine_log" >&2 || true
      timeout 120 bash -c 'engine shutdown' || true
      exit 1
    fi
    echo "clipboard history recorded both markers:"
    grep 'clipboard change' "$engine_log"
    timeout 120 bash -c 'engine shutdown' || true
    ;;

  # Spike B (#3): the same question on the target kernel. The Flatpak CI job
  # answers it in three minutes on the runner's kernel; this one answers it on
  # Bluefin's, which is what actually ships, and Landlock's ABI is a kernel
  # property. Evidence only — no assertion, because every outcome is a finding.
  spike-b)
    u="$(uid)"
    runuser -u "$SESSION_USER" -- env \
      XDG_RUNTIME_DIR="/run/user/$u" \
      DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$u/bus" \
      flatpak run --installation="$INSTALLATION" "$APP" spike sandbox
    ;;

  # ── Spike A ────────────────────────────────────────────────────────────────
  #
  # Two halves, because a host-side keypress has to happen between them. corral
  # runs every --check over its own SSH connection and cannot interleave a host
  # command, so Spike A is driven by scripts/vmtest/spike-a.sh after vmtest
  # returns, against a VM left running.

  # Start the spike detached and return once it says it is listening. Returning
  # earlier would race: binding is a portal round trip and, on a first run, a
  # permission dialog, and a key sent before the bind lands proves nothing.
  spike-a-start)
    u="$(uid)"
    # Created empty rather than removed: the waiter greps the stderr file, and
    # a file that does not exist yet makes grep print "No such file or
    # directory" into a log where it reads like the failure rather than like
    # the first poll of a loop that then succeeded.
    : > "$SPIKE_OUT"
    : > "$SPIKE_ERR"
    rm -f "$SPIKE_DONE"

    # setsid and all three fds redirected: without that, ssh waits for the
    # channel to close and this check never returns.
    #
    # The wrapper exists to write $SPIKE_DONE when the spike exits. The obvious
    # alternative — having the collector poll `pgrep -f "spike global-shortcut"`
    # — cannot work, and failed exactly this way: pgrep matches full command
    # lines, so the shell running the pgrep contains the pattern and matches
    # itself. The predicate is then never true and the wait always times out.
    # A sentinel file has no such reflexivity, and it carries the exit status.
    #
    # Arguments are passed positionally rather than interpolated, so nothing
    # here depends on quoting surviving two levels of shell.
    setsid bash -c '
      runuser -u "$1" -- env \
        XDG_RUNTIME_DIR="/run/user/$2" \
        DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$2/bus" \
        WAYLAND_DISPLAY="$3" \
        XDG_SESSION_TYPE=wayland \
        flatpak run --installation="$4" "$5" \
          spike global-shortcut --trigger "$6" --wait 120 --json \
        > "$7" 2> "$8"
      echo "$?" > "${9}"
    ' _ "$SESSION_USER" "$u" "$(wayland_display)" "$INSTALLATION" "$APP" \
      "${2:-SUPER+space}" "$SPIKE_OUT" "$SPIKE_ERR" "$SPIKE_DONE" \
      < /dev/null >> "$SPIKE_ERR" 2>&1 &

    # Wait for the marker OR the spike exiting, not the marker alone. A spike
    # that cannot reach the portal writes its report and exits, and waiting only
    # for the marker means sitting out the full timeout and then discarding an
    # answer that already existed. (The spike now announces readiness on every
    # path, so this is belt and braces — but the belt is what turns a hang into
    # a report, and it costs one `-f` test.)
    wait_for "the spike to bind and start listening, or exit" 150 \
      bash -c 'grep -q SPIKE-A-READY "$1" || [ -f "$2" ]' _ "$SPIKE_ERR" "$SPIKE_DONE"

    if ! grep -q SPIKE-A-READY "$SPIKE_ERR"; then
      echo "the spike exited before announcing readiness; its report follows in the next step"
    fi
    cat "$SPIKE_ERR"
    ;;

  # Wait for the spike to finish — it exits on the first activation, or at its
  # own deadline — and print the report. Deliberately does NOT assert that the
  # shortcut fired: "GNOME refused to bind without a click nobody can give" is
  # an answer to the question, not a broken run, and a gate here would turn the
  # finding into a red X with no information in it.
  spike-a-collect)
    wait_for "the spike to finish" 180 test -f "$SPIKE_DONE"
    echo "the spike exited $(cat "$SPIKE_DONE")"
    echo '--- stderr ---'
    cat "$SPIKE_ERR" 2>/dev/null || echo '(none)'
    echo '--- report ---'
    cat "$SPIKE_OUT"
    python3 -c "import json,sys; json.load(open('$SPIKE_OUT'))" \
      || { echo 'the spike produced no valid JSON report' >&2; exit 1; }
    ;;

  # Does the compositor have the keyboard open?
  #
  # The evdev capture settled that scancodes reach the kernel — 288 bytes, 12
  # input_event structs, exactly two press/release pairs. And they reach
  # neither GNOME's Super binding nor a focused application: typing into the
  # launcher's own text input left the frame byte-identical.
  #
  # Keys arriving at the kernel and being acted on by nobody points at one
  # thing worth checking before any theory: whether gnome-shell has the device
  # open at all. libinput reads evdev nodes directly, so if the compositor's
  # process holds no /dev/input fd, it is not reading the keyboard and nothing
  # above that matters.
  #
  # This is a fact, not an inference, and it costs one readlink loop.
  compositor-input)
    shell_pid="$(pgrep -u "$SESSION_USER" -x gnome-shell | head -1 || true)"
    if [ -z "$shell_pid" ]; then
      echo "no gnome-shell process for $SESSION_USER" >&2
      exit 1
    fi
    echo "gnome-shell pid $shell_pid"

    echo '--- input devices gnome-shell holds open ---'
    found=0
    for fd in "/proc/$shell_pid/fd"/*; do
      [ -e "$fd" ] || continue
      target="$(readlink "$fd" 2>/dev/null || true)"
      case "$target" in
        /dev/input/*) printf '  %s -> %s\n' "$(basename "$fd")" "$target"; found=1 ;;
      esac
    done
    if [ "$found" = 0 ]; then
      echo '  NONE — the compositor is not reading any input device.'
      echo '  That is the answer: keys reach the kernel and nobody is listening.'
    fi

    # logind hands input devices to the active session through TakeDevice, so
    # if the compositor has none, its view of the seat is where to look next.
    #
    # This used to print `seat-status | sed -n '1,25p'`, which truncated the
    # device tree before any input device appeared -- the first 25 lines are
    # the DRM card, the optical drive, i2c and the power button. The one
    # question being asked, "is the KEYBOARD on this seat", was exactly what
    # got cut off. Filtered to input devices instead of arbitrarily truncated.
    echo '--- what logind thinks the seat has (input devices only) ---'
    if seat="$(loginctl seat-status seat0 2>/dev/null)"; then
      printf '%s\n' "$seat" | sed -n '1,3p'
      inputs="$(printf '%s\n' "$seat" | grep -E '/input/input[0-9]+$' || true)"
      if [ -n "$inputs" ]; then
        printf '%s\n' "$inputs" | sed 's/^/  /'
      else
        echo '  NONE — seat0 has no input devices assigned.'
      fi

      # The keyboard specifically. i8042 is the AT controller the emulated
      # "AT Translated Set 2 keyboard" hangs off; if the compositor reads a
      # device logind has not put on this seat, that mismatch is the lead.
      if printf '%s\n' "$seat" | grep -q 'i8042'; then
        echo '  -> the AT keyboard IS on seat0'
      else
        echo '  -> the AT keyboard is NOT on seat0 (i8042 absent from the tree)'
      fi
    else
      echo '(seat-status unavailable)'
    fi

    # IS THE SESSION ACTIVE?
    #
    # This is the question the rest of the evidence now points at. Everything
    # else is green: the keyboard is on seat0, the kernel receives clean
    # LEFTMETA press and release on event1, and gnome-shell holds event1 open.
    # Yet pressing Super alone leaves the framebuffer byte-identical -- and
    # Super alone is GNOME's OWN binding for the Activities overview, nothing
    # to do with our portal shortcut. So the compositor is inert to this input
    # generally, not failing to route one shortcut.
    #
    # logind pauses a session's input devices when the session is not active,
    # and a paused device keeps its file descriptor open -- the fd is how the
    # resume is delivered. So "gnome-shell holds event1" is entirely consistent
    # with gnome-shell receiving nothing from it, and Active= is what tells the
    # two apart.
    echo '--- is the session active? (paused devices keep their fds) ---'
    sid="$(loginctl list-sessions --no-legend 2>/dev/null | awk -v u="$SESSION_USER" '$3 == u {print $1; exit}')"
    if [ -n "$sid" ]; then
      loginctl show-session "$sid" \
        -p Id -p User -p Name -p Seat -p Type -p Class -p State -p Active -p Remote \
        2>/dev/null | sed 's/^/  /'
    else
      echo "  no logind session found for $SESSION_USER"
      loginctl list-sessions --no-legend 2>/dev/null | sed 's/^/  /' || true
    fi
    ;;

  # Does an injected scancode reach the guest KERNEL?
  #
  # This splits the one question left about Spike A. Pressing Super alone
  # changes nothing on screen, and the guest does have a keyboard — the
  # evidence check shows an "AT Translated Set 2 keyboard" with an evdev node,
  # so the earlier guess that corral adds no input device was wrong. That
  # leaves two possibilities that look identical from outside:
  #
  #   a. QEMU never delivers the scancode to the emulated keyboard, or
  #   b. it does, and something above the kernel — mutter, the seat, focus —
  #      discards it.
  #
  # Reading the evdev node decides it. Bytes arriving while the host injects a
  # key means the kernel got the event and (b) is the answer; silence means (a),
  # and it is corral's or QEMU's to fix rather than ours.
  #
  # The node is resolved from /proc/bus/input/devices rather than hard-coded to
  # event1: it is event1 today, and a hard-coded node that silently moves would
  # report "no input" for a keyboard that is working perfectly.
  keyboard-capture-start)
    node="$(awk '
      /^N: Name=/ { name=$0 }
      /^H: Handlers=/ { handlers=$0 }
      /^B: EV=/ {
        if (name ~ /[Kk]eyboard/ && handlers ~ /event[0-9]+/) {
          match(handlers, /event[0-9]+/)
          print substr(handlers, RSTART, RLENGTH)
        }
        name=""; handlers=""
      }' /proc/bus/input/devices | head -1)"
    if [ -z "$node" ]; then
      echo "no keyboard evdev node found; /proc/bus/input/devices follows" >&2
      cat /proc/bus/input/devices >&2
      exit 1
    fi
    echo "capturing from /dev/input/$node"
    rm -f "$KBD_CAP" "$KBD_CAP_DONE"
    # timeout, not a kill later: the capture must end on its own even if the
    # collector never runs, or a failed run leaves a cat holding the device.
    setsid bash -c '
      timeout 25 cat "/dev/input/$1" > "$2" 2>/dev/null
      echo done > "$3"
    ' _ "$node" "$KBD_CAP" "$KBD_CAP_DONE" < /dev/null > /dev/null 2>&1 &
    # A moment for the redirect to actually open the device, so a key pressed
    # immediately afterwards is not injected into a capture that has not started.
    sleep 2
    ;;

  # Read back what the kernel saw. Run after the host has injected the key.
  keyboard-capture-report)
    wait_for "the keyboard capture to finish" 40 test -f "$KBD_CAP_DONE"
    bytes=$(wc -c < "$KBD_CAP" 2>/dev/null || echo 0)
    echo "captured $bytes bytes of input events while the key was injected"

    # Decode them rather than trusting the byte count. 288 bytes is exactly
    # what two press/release pairs should produce, which is suggestive and is
    # not proof: autorepeat, a stray mouse event or a jittering power button
    # would also make bytes. Printing the keycodes says what actually arrived.
    #
    # struct input_event on 64-bit is two __kernel_ulong_t of timeval, then
    # __u16 type, __u16 code, __s32 value — 24 bytes, "qqHHi".
    python3 - "$KBD_CAP" <<'PY' || echo '(decode failed; the byte count above stands)' 
import struct, sys

SIZE = struct.calcsize("qqHHi")
EV_KEY = 0x01
NAMES = {1: "ESC", 57: "SPACE", 125: "LEFTMETA", 126: "RIGHTMETA"}
ACTION = {0: "release", 1: "press", 2: "autorepeat"}

data = open(sys.argv[1], "rb").read()
if len(data) % SIZE:
    print(f"  warning: {len(data)} bytes is not a whole number of {SIZE}-byte events")

keys = []
for off in range(0, len(data) - SIZE + 1, SIZE):
    _sec, _usec, etype, code, value = struct.unpack("qqHHi", data[off : off + SIZE])
    if etype == EV_KEY:
        keys.append((code, value))
        print(f"  EV_KEY code={code} ({NAMES.get(code, 'other')}) "
              f"value={value} ({ACTION.get(value, '?')})")

print(f"  {len(data) // SIZE} events total, {len(keys)} of them key events")
if not keys:
    print("  NOTE: bytes arrived but none were key events — the count alone "
          "would have been misleading")
PY

    if [ "$bytes" -gt 0 ]; then
      echo "VERDICT: the scancode REACHED the guest kernel — the loss is above it"
    else
      echo "VERDICT: NOTHING reached the guest kernel — QEMU never delivered it"
    fi
    ;;

  # Evidence for Spike A, gathered before the spike runs so that a hang has
  # something to be read against. Nothing here asserts: each line is a fact the
  # report needs in order to be interpretable, and a missing fact is itself
  # worth seeing.
  #
  # Two questions, and they fail in ways that look identical from the client:
  #
  #   1. Did the pre-seed land? If the system dconf database did not compile,
  #      or the profile does not reference it, the grant is simply absent and
  #      BindShortcuts hangs at the consent dialog exactly as it did before.
  #      Read as the session user, through the same dconf profile GNOME uses —
  #      reading the keyfile in /etc would only prove we wrote a file.
  #   2. Does Super+Space already belong to something else? GNOME binds it to
  #      the input-source switcher by default. A collision does not stop the
  #      bind; it means the compositor may route the key elsewhere, so
  #      "activated: false" would mean "someone else got the key" rather than
  #      "the portal does not deliver". Those are different answers and the
  #      spike cannot tell them apart on its own.
  spike-a-evidence)
    u="$(uid)"
    as_session() {
      runuser -u "$SESSION_USER" -- env \
        XDG_RUNTIME_DIR="/run/user/$u" \
        DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$u/bus" \
        "$@"
    }

    echo '--- dconf profile ---'
    cat /etc/dconf/profile/user 2>/dev/null || echo '(no profile — the seed cannot be visible)'
    echo '--- compiled system database ---'
    ls -l /etc/dconf/db/compass 2>/dev/null || echo '(not compiled — dconf update did not run or found nothing)'

    echo '--- the seeded grant, as the session user sees it ---'
    as_session dconf read \
      "/org/gnome/settings-daemon/global-shortcuts/$APP/shortcuts" \
      || echo '(unreadable)'
    as_session dconf read \
      /org/gnome/settings-daemon/global-shortcuts/applications || true

    # Is there a keyboard at all? Measured after the first run where an
    # injected key changed nothing: corral builds its QEMU command line with
    # `-vga virtio -display none` and adds virtio net and rng devices, but no
    # input device. On x86 the default machine still provides a PS/2 controller,
    # so a keyboard *should* be here — "should" being exactly the word that
    # earned this check. If /proc/bus/input/devices lists no keyboard, the
    # scancodes have nowhere to arrive and no amount of portal work matters.
    echo '--- input devices the kernel knows about ---'
    grep -iE '^[NHB]: (Name|Handlers|EV)' /proc/bus/input/devices 2>/dev/null \
      || echo '(no /proc/bus/input/devices — no input subsystem at all)'
    echo '--- and what libinput sees, if it is installed ---'
    as_session libinput list-devices 2>/dev/null | grep -iE 'Device:|Capabilities:' \
      || echo '(libinput not available in the session; the kernel list above is the evidence)'

    echo '--- which GNOME this is (Phase 1 gates on 50 and 51) ---'
    as_session gnome-shell --version 2>/dev/null || echo '(gnome-shell --version unavailable)'
    grep -E '^(NAME|VERSION|VERSION_ID)=' /etc/os-release 2>/dev/null || true

    echo '--- who else wants Super+Space ---'
    # Not exhaustive and not meant to be: these are the two schemas whose
    # defaults actually collide on this combination. Anything else that claims
    # it will show up as a keypress that never arrives, which is why the raw
    # spike report stays the primary evidence.
    as_session gsettings get org.gnome.desktop.wm.keybindings switch-input-source 2>/dev/null || true
    as_session gsettings get org.gnome.desktop.wm.keybindings switch-input-source-backward 2>/dev/null || true
    as_session gsettings get org.gnome.shell.keybindings toggle-overview 2>/dev/null || true
    ;;

  # Open the launcher in the session and leave it open.
  #
  # This is the first check that exercises the product rather than the platform
  # under it. Everything above answers "can compass run here"; this answers
  # "does compass draw a launcher here", which is the question the tier was
  # built for and could not ask until there was a launcher to open.
  #
  # Same setsid + sentinel shape as spike-a-start, and for the same reasons:
  # without detaching and redirecting all three fds, ssh waits for the channel
  # and the check never returns; and a sentinel file carries the exit status
  # without the `pgrep -f` self-match that made an earlier waiter here time out
  # every single time.
  #
  # Unlike the spike, this process is meant to *stay running* — the sentinel
  # appearing at all is the failure, not the success.
  launcher-start)
    u="$(uid)"
    : > "$UI_ERR"
    rm -f "$UI_DONE"

    # §8.5's last unmeasured row is "Cold start to first frame < 120 ms", and it
    # had no measurement anywhere. This is the closest thing the guest can
    # honestly produce, and the gap between it and the SLA is stated below
    # rather than glossed.
    start_ms="$(date +%s%3N)"

    # RUST_LOG and the wgpu/winit knobs are set because the first run of this
    # job produced a launcher that started, stayed alive, exited nothing, and
    # printed *not one line* — while never putting a window on screen. A silent
    # failure is the worst kind to debug from a 25-minute VM job, so the next
    # run is made to talk.
    #
    # WGPU_BACKEND is deliberately not pinned to a value: naming one would
    # decide the answer instead of measuring it. What is wanted is wgpu's own
    # account of which adapters it found under llvmpipe, which `info` gives.
    setsid bash -c '
      runuser -u "$1" -- env \
        XDG_RUNTIME_DIR="/run/user/$2" \
        DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$2/bus" \
        WAYLAND_DISPLAY="$3" \
        XDG_SESSION_TYPE=wayland \
        RUST_LOG="info,compass_ui::state=debug,wgpu=debug,wgpu_hal=debug,iced_wgpu=debug,\
winit=debug,sctk_adwaita=debug,smithay_client_toolkit=debug,wayland_client=debug,calloop=debug" \
        RUST_BACKTRACE=1 \
        flatpak run --env=COMPASS_NO_ONBOARDING=1 --installation="$4" "$5" ui \
        > "$6" 2>&1
      echo "$?" > "$7"
    ' _ "$SESSION_USER" "$u" "$(wayland_display)" "$INSTALLATION" "$APP" \
      "$UI_ERR" "$UI_DONE" \
      < /dev/null >> "$UI_ERR" 2>&1 &

    # Nothing inside the guest can observe "has painted" — ADR-0010 says so and
    # it is still true: the framebuffer's only observer is corral, on the far
    # side of QEMU. So what is waited on here is the weaker but real thing, the
    # app process existing under the session user. A launcher that dies on
    # startup — the likeliest failure, and one that would otherwise surface as
    # an inscrutable unchanged frame — fails here instead, with its own output
    # attached. The host screenshot is what closes the gap between "the process
    # is alive" and "a launcher is on screen"; neither half is sufficient.
    #
    # `pgrep -u ... -x` matches the process *name*, deliberately, not `-f`
    # against the command line: an `-f` pattern distinctive enough to find this
    # process is also present in the command line of the shell doing the
    # matching, so the predicate matches itself and is true before the launcher
    # has done anything at all. That exact bug cost an earlier waiter here 180
    # seconds a run, and it presents as a timeout rather than as a mistake.
    wait_for "the launcher process to appear, or exit" 90 launcher_appeared
    spawned_ms="$(date +%s%3N)"

    # And then wait for it to be READY, which is not the same thing and cost
    # three runs and a wrong conclusion to learn.
    #
    # The process existing says nothing about whether anything is on screen.
    # Under llvmpipe this launcher takes seconds to first paint and the spread
    # is wide: one run had wgpu initialising 2.4s after start, another had not
    # touched wgpu 8.1s in. Screenshotting at process-appear caught the second
    # kind twice and produced "the launcher does not draw", which was wrong.
    #
    # ADR-0010's own rule is to key off a state and never a duration, and this
    # check was breaking it. `Adapter AdapterInfo` in the launcher's own log is
    # a real state: wgpu only reports a chosen adapter once it has a surface to
    # render to. The settle after it is slack after a state, the same shape and
    # the same justification as wait-graphical.sh.
    wait_for "the renderer to choose an adapter, or the launcher to exit" 150 \
      bash -c 'grep -q "Adapter AdapterInfo" "$1" || [ -f "$2" ]' \
      _ "$UI_ERR" "$UI_DONE"
    ready_ms="$(date +%s%3N)"

    # WHAT THIS NUMBER IS, AND WHAT §8.5 ASKED FOR.
    #
    # The SLA is "cold start to first frame". Nothing inside the guest can
    # observe a frame — ADR-0010 settles that, and it is why the paint gate
    # lives on the host with corral's screenshots. So this measures the nearest
    # state the guest CAN see: `Adapter AdapterInfo`, which wgpu emits only once
    # it has a surface to render to. First paint follows shortly after.
    #
    # It is REPORTED, NOT GATED, for the same reason §11.2 reports RSS rather
    # than gating it. Under llvmpipe on a QEMU guest this is software rendering
    # on emulated hardware, and the spread is enormous — ADR-0010 records 2.4 s
    # to wgpu in one run against not-yet at 8.1 s in another. A 120 ms budget
    # measured here would be measuring the VM, not the launcher, and gating on
    # it would make the tier red for reasons that have nothing to do with the
    # code.
    #
    # The split matters too: spawn cost is Flatpak and process start, render
    # cost is wgpu bringing up a software adapter. Only the second is what the
    # SLA is about, and only the first would improve on real hardware.
    echo "cold start: $((spawned_ms - start_ms)) ms to process, \
$((ready_ms - spawned_ms)) ms process to renderer-ready, \
$((ready_ms - start_ms)) ms total (llvmpipe, reported not gated — see §8.5)"

    sleep 3

    if [ -f "$UI_DONE" ]; then
      echo "the launcher exited $(cat "$UI_DONE") instead of staying open; its output follows" >&2
      cat "$UI_ERR" >&2
      exit 1
    fi
    echo "the launcher is running; output so far:"
    cat "$UI_ERR"
    ;;

  # What is the launcher actually blocked on?
  #
  # The instrumented run answered one question and posed a sharper one: the log
  # ends 200 ms in, at an `sctk_adwaita` XDG-Settings-Portal timeout inside
  # `create_window`, and then nothing — no wgpu lines at all, though wgpu,
  # wgpu_hal and iced_wgpu were all at debug. So execution never reaches wgpu,
  # and the process sits there alive for the rest of the run.
  #
  # Logs cannot say where it stopped, because the stuck code is not logging.
  # The kernel can. /proc/PID/wchan and /proc/PID/syscall name the syscall a
  # thread is parked in, and the open fds say which socket it is parked on —
  # a Wayland socket and a D-Bus socket look nothing alike here. No debugger
  # needed, nothing to install, and it works on a stripped image.
  #
  # Reading the host's /proc for a Flatpak process is fine: bwrap namespaces
  # the guest's view, not root's.
  launcher-diagnose)
    pid="$(launcher_pid || true)"
    if [ -z "$pid" ]; then
      echo "no compass process to diagnose"
      exit 0
    fi
    echo "pid $pid"
    echo "--- state ---"
    grep -E '^(State|Threads|SigBlk):' "/proc/$pid/status" 2>/dev/null || true
    echo "--- per-thread: what each one is parked in ---"
    # The main thread is the interesting one, but a hang in a worker looks
    # identical from outside, so all of them are printed.
    for t in /proc/"$pid"/task/*; do
      tid="$(basename "$t")"
      printf '  tid %s  state=%s  wchan=%s\n' \
        "$tid" \
        "$(awk '/^State:/{print $2}' "$t/status" 2>/dev/null)" \
        "$(cat "$t/wchan" 2>/dev/null || echo '?')"
      printf '    syscall: %s\n' "$(cat "$t/syscall" 2>/dev/null || echo '?')"
    done
    echo "--- sockets it holds open ---"
    # readlink over a glob rather than `ls -l | grep`: shellcheck SC2010 is
    # right that parsing ls breaks on odd names, and /proc/PID/fd is exactly
    # where symlink targets get interesting.
    found=0
    for fd in "/proc/$pid/fd"/*; do
      [ -e "$fd" ] || continue
      target="$(readlink "$fd" 2>/dev/null || true)"
      case "$target" in
        socket:*|*wayland*|*dbus*|*bus)
          printf '  %s -> %s\n' "$(basename "$fd")" "$target"; found=1 ;;
      esac
    done
    [ "$found" = 1 ] || echo '(no sockets listed)'
    echo "--- and which of those the kernel can name ---"
    # The first version of this printed "(ss unavailable or no match)", which
    # conflates a missing tool with a tool that found nothing — and those want
    # different fixes. It reported exactly that, uselessly. Say which.
    if ! command -v ss >/dev/null 2>&1; then
      echo '(ss is not in this image; falling back to /proc/net/unix below)'
    elif ! ss -x -p 2>/dev/null | grep -F "pid=$pid"; then
      echo '(ss ran and matched no socket for this pid)'
    fi

    # The fallback, which needs no tools at all. Note its limit up front: a
    # *connected* AF_UNIX socket has an empty path column, so this names the
    # listening sockets and leaves client ends as "(unnamed)". Verified on this
    # machine before shipping rather than discovered in the guest.
    echo '--- socket inodes against /proc/net/unix ---'
    for fd in "/proc/$pid/fd"/*; do
      [ -e "$fd" ] || continue
      target="$(readlink "$fd" 2>/dev/null || true)"
      case "$target" in
        socket:*)
          ino="${target#socket:[}"; ino="${ino%]}"
          path="$(awk -v i="$ino" '$7==i {print ($8==""?"(unnamed — connected end)":$8)}' \
                  /proc/net/unix 2>/dev/null)"
          printf '  fd %s inode %s -> %s\n' \
            "$(basename "$fd")" "$ino" "${path:-(not listed)}" ;;
      esac
    done
    ;;

  # Phase 1's gate says "idle RSS < 30 MB". Nobody had measured it, because
  # until the launcher drew there was nothing to measure.
  #
  # Read after the window is up and the run has gone quiet, which is what
  # "idle" means here: the renderer has an adapter, the first frame is done,
  # and nothing is being typed. RSS is taken from the host's /proc — a Flatpak
  # process is an ordinary process to the kernel, and VmRSS is the number the
  # gate is phrased in.
  #
  # Reported, not gated. A number measured once is not a budget, and a memory
  # gate set from a single sample on a software-rendered VM would be the
  # deviation mistake again in a different costume. It goes in the log so the
  # gate can be set from a distribution later.
  launcher-rss)
    pid="$(launcher_pid || true)"
    if [ -z "$pid" ]; then
      echo "no launcher process to measure" >&2
      exit 1
    fi
    rss_kb="$(awk '/^VmRSS:/{print $2}' "/proc/$pid/status" 2>/dev/null || echo 0)"
    hwm_kb="$(awk '/^VmHWM:/{print $2}' "/proc/$pid/status" 2>/dev/null || echo 0)"
    printf 'launcher pid %s: VmRSS %s kB (%s MB), peak VmHWM %s kB (%s MB)\n' \
      "$pid" "$rss_kb" "$((rss_kb / 1024))" "$hwm_kb" "$((hwm_kb / 1024))"
    # THE ENGINE IS MEASURED TOO, AND IT IS THE MORE HONEST NUMBER.
    #
    # Phase 1's gate says "idle RSS < 30 MB" without naming a process, and this
    # check used to answer it with the window's figure alone -- ~135 MB, read as
    # our code being five times over budget. Most of that is wgpu's software
    # renderer: under llvmpipe the GPU stack lives in this process's RSS, and on
    # hardware it does not.
    #
    # The engine is the part that is actually resident. It holds the index and
    # serves IPC, it runs whether or not a window is open, and it draws nothing,
    # so its RSS is comparable across a VM and a real machine. Measured on an
    # ordinary x86-64 container outside any VM it idles at 6.2 MB.
    #
    # Both are printed, labelled, and neither is gated -- the numbers decide the
    # threshold, not the other way round (ADR-0010).
    engine_pid="$(head -1 "$ENGINE_PIDS" 2>/dev/null || true)"
    if [ -n "$engine_pid" ] && [ -r "/proc/$engine_pid/status" ]; then
      erss_kb="$(awk '/^VmRSS:/{print $2}' "/proc/$engine_pid/status" 2>/dev/null || echo 0)"
      printf 'engine   pid %s: VmRSS %s kB (%s MB) -- resident, draws nothing\n' \
        "$engine_pid" "$erss_kb" "$((erss_kb / 1024))"
    else
      erss_kb=""
    fi

    printf 'Phase 1 gate is "idle RSS < 30 MB".\n'
    printf '  launcher %s MB — recorded, not gated\n' "$((rss_kb / 1024))"
    # Under llvmpipe the renderer keeps its own buffers, so a VM number is not
    # a hardware number. Said here so nobody reads it as one.
    echo 'note: the launcher figure includes wgpu under software rendering, so it is'
    echo '      an upper bound rather than the shipping figure; the engine figure is not.'

    # The same figures, machine-readable, for `perf-gate.py extract` (#307).
    # Two names, because the window's figure is mostly the renderer and is
    # only comparable with other llvmpipe readings, while the engine's is not.
    # A zero is an unreadable /proc, not a reading, so it is not emitted.
    if [ "${rss_kb:-0}" -gt 0 ]; then
      printf 'perf-point window.idle_rss_kb %s kB\n' "$rss_kb"
    fi
    if [ "${erss_kb:-0}" -gt 0 ]; then
      printf 'perf-point engine.idle_rss_kb %s kB\n' "$erss_kb"
    fi

    # THE ENGINE FIGURE IS NOW GATED, AND THE CEILING COMES FROM MEASUREMENT.
    #
    # ADR-0010: recorded before gated. It has been recorded, and the numbers
    # are steady --
    #
    #   10,064 kB (9 MB)   run on 4254ba3
    #   10,484 kB (10 MB)  run on f6bc770
    #    ~6,300 kB (6.2 MB) on an ordinary x86-64 container, no VM
    #
    # -- a 4% spread across two VM runs, so a ceiling is defensible where an
    # equality would not be.
    #
    # ENGINE_RSS_CEILING_MB is 20: about double the measured value, and
    # two-thirds of the #13 SLA. Deliberately not tight to the measurement,
    # because the index grows with the image's application set and somebody
    # else's Bluefin rebase must not turn our run red. Deliberately not 30
    # either -- sitting on the SLA would let the engine triple before anything
    # complained, which is the decay this gate exists to prevent.
    #
    # Only the engine is gated. The launcher's figure is mostly llvmpipe, so
    # gating it would gate Mesa.
    ENGINE_RSS_CEILING_MB=20
    if [ -z "$erss_kb" ]; then
      # Not a pass. A memory gate that quietly skips when it cannot find its
      # process reports the same green as one that measured and was satisfied.
      echo "FAIL: no engine pid recorded, so the idle-RSS gate did not run" >&2
      exit 1
    fi
    engine_mb=$((erss_kb / 1024))
    # A floor as well as a ceiling, and it is not defensive padding: the read
    # above is `awk ... || echo 0`, so an unreadable /proc entry -- or a process
    # that exits between the existence check and the read -- yields 0, and 0 is
    # comfortably under any ceiling. An engine reporting no resident memory is a
    # failed measurement, not a lean engine, and it must not report green. This
    # is the same defect the index floor had when it asserted `-gt 0`.
    if [ "$erss_kb" -lt 1024 ]; then
      echo "FAIL: engine idle RSS read as ${erss_kb} kB, which is not a measurement" >&2
      echo "      /proc/$engine_pid/status was unreadable, or the engine exited." >&2
      exit 1
    fi
    if [ "$engine_mb" -ge "$ENGINE_RSS_CEILING_MB" ]; then
      echo "FAIL: engine idle RSS is ${engine_mb} MB, at or over the ${ENGINE_RSS_CEILING_MB} MB ceiling" >&2
      echo "      (measured at 9-10 MB when this ceiling was set; the #13 SLA is 30 MB)" >&2
      echo "      Find what grew. Do not raise the ceiling to fit." >&2
      exit 1
    fi
    printf '  engine   %s MB — under the %s MB ceiling (SLA 30 MB)\n' \
      "$engine_mb" "$ENGINE_RSS_CEILING_MB"
    ;;

  # Harvest a real desktop-entry corpus from this Bluefin box.
  #
  # Phase 1's gate wants ~500 real entries for Suite 0 ranking parity and there
  # are 27 (§11.2). The harvester's own header asks for "a real desktop —
  # ideally a Bluefin box", and that is exactly what this VM is: a full GNOME
  # application set on the target platform, booted fresh every run.
  #
  # It writes into a scratch directory rather than the repo's corpus path,
  # because nothing here should look like it commits to the repository. The
  # tarball goes out with the artifacts and a person decides what to keep — a
  # corpus is test input that shapes every ranking assertion, and it should not
  # grow by a job quietly appending to it.
  # Are the machine's Flatpak applications in the index? (#105)
  #
  # A Flatpak export is a SYMLINK, not a file. flatpak-dir.c builds it with the
  # prefix `../app/<id>/current/active/export`, so the entry a launcher reads
  # lives in the deploy tree and the exports directory holds only pointers into
  # it. Our sandbox granted the exports directories and not the deploy trees,
  # which meant every Flatpak on the machine was a dangling link inside it --
  # invisible, with no error anywhere, because the scan still lists the name and
  # the read fails one step later.
  #
  # Unit tests pin the shape of that (compass-xdg) and the manifest invariant
  # (xdg_dirs), but only a real session can answer whether the sandbox actually
  # resolves them. That is what this is: two applications installed by flatpak
  # itself, one in the system root and one in the user's, queried through the
  # running engine from inside the Flatpak.
  #
  # The display name is read from the export rather than written here, so the
  # check does not break when an upstream renames its application; the app id is
  # what is asserted, because that is what the index keys on.
  flatpak-apps)
    # THE SYSTEM ROOT IS GATED; THE USER ROOT IS RECORDED. The image installs a
    # probe application into the system root only: a container build is the
    # wrong place to populate a USER installation, and two attempts proved it
    # (gpgme has no session to work in, and flatpak refuses `--user` as root).
    # The follow-up installs it in the booted guest from a staged bundle, where
    # there is a real session.
    #
    # So an empty user root is reported as UNCOVERED and does not fail, while a
    # user root that HAS an export is gated exactly like the system one -- the
    # day the follow-up lands, this starts gating it with no change here. What
    # is never allowed is silence: a root with nothing in it says so, loudly,
    # every run.
    status=0
    for spec in \
      "system:/var/lib/flatpak/exports/share/applications:gated" \
      "user:/var/home/$SESSION_USER/.local/share/flatpak/exports/share/applications:recorded"
    do
      root="${spec%%:*}"
      rest="${spec#*:}"
      dir="${rest%%:*}"
      mode="${rest##*:}"

      if [ ! -d "$dir" ]; then
        if [ "$mode" = recorded ]; then
          echo "UNCOVERED: $root: $dir does not exist; no probe app is installed in this root yet"
          continue
        fi
        echo "FAIL: $root: $dir does not exist, so the image never installed a probe app" >&2
        status=1
        continue
      fi

      found=0
      for entry in "$dir"/*.desktop; do
        # `-e` follows the link, so a DANGLING export fails it and the glob's
        # own literal fallback fails it too. Testing `-L` as well keeps the two
        # apart: a dangling link here is a broken image and must be reported as
        # one, not skipped into "no exports at all", which is what the first
        # draft of this did.
        [ -e "$entry" ] || [ -L "$entry" ] || continue
        found=1
        app_id="$(basename "$entry" .desktop)"

        if [ ! -L "$entry" ]; then
          echo "note: $root: $app_id is not a symlink; flatpak's export layout has changed" >&2
        elif [ ! -e "$entry" ]; then
          # Note this is the view from OUTSIDE the sandbox, as root. #105 is a
          # link that dangles only *inside* it, which no check here can see --
          # the query below is what answers that. A link broken out here is a
          # different and worse thing: the image itself is wrong.
          echo "FAIL: $root: $app_id is a dangling symlink on the host: $(readlink "$entry")" >&2
          echo "  the image installed it and then lost the deploy tree; this is not #105" >&2
          status=1
          continue
        fi

        name="$(sed -n 's/^Name=//p' "$entry" | head -1)"
        if [ -z "$name" ]; then
          echo "FAIL: $root: no Name= in $entry" >&2
          status=1
          continue
        fi

        echo "--- $root: $app_id ($name) ---"
        if compass_cli query --json "$name" \
             > /tmp/flatpak-query.json 2>/tmp/flatpak-query.err
        then
          APP_ID="$app_id" NAME="$name" ROOT="$root" python3 - <<'PY' || status=1
import json, os, sys

app_id = os.environ["APP_ID"]
name = os.environ["NAME"]
root = os.environ["ROOT"]
hits = json.load(open("/tmp/flatpak-query.json"))
for hit in hits[:5]:
    print(f"  {hit['score']:3} {hit['id']}  {hit['title']}")
# The wire carries the ENTRYPOINT id (`applications:<desktop id>`, see
# serve.rs app_hit), not the desktop file name. The bare forms are kept so a
# run against an older engine reads the same.
if any(hit["id"] in (f"applications:{app_id}", f"{app_id}.desktop", app_id) for hit in hits):
    print(f"  ok: the {root} Flatpak {app_id} is in the index")
    sys.exit(0)
sys.exit(
    f"FAIL: querying {name!r} did not return the {root} Flatpak {app_id}.\n"
    "  Its exported .desktop is a symlink into the deploy tree; if the sandbox "
    "cannot read that tree the entry is invisible and every Flatpak on the "
    "machine disappears from search (#105).\n"
    "  Check --filesystem=/var/lib/flatpak/app:ro and "
    "--filesystem=xdg-data/flatpak/app:ro in the manifest."
)
PY
        else
          echo "FAIL: $root: the query itself failed" >&2
          cat /tmp/flatpak-query.err >&2
          status=1
        fi
      done

      if [ "$found" -eq 0 ]; then
        if [ "$mode" = recorded ]; then
          echo "UNCOVERED: $root: no .desktop exports in $dir; nothing installed in this root yet"
        else
          echo "FAIL: $root: no .desktop exports in $dir; the image installed no probe app" >&2
          status=1
        fi
      fi
    done
    exit "$status"
    ;;

  # Everything systemd knows that a failing gate would want.
  #
  # This tier had no journal at all: `launcher-diagnose` reads /proc for a
  # process that is hung, which says nothing about a unit that never started, a
  # portal that exited, or a session that came up wrong. Adapted from tunaOS's
  # `iso-e2e.sh`, which learned the shape of this the expensive way; the three
  # lessons worth keeping are marked below.
  journal)
    echo '=== failed units ==='
    systemctl --failed --no-pager --full || true

    # LESSON 1: every failed unit, not a hardcoded list. The unit that matters
    # is the FIRST one to fail, and which one that is differs per failure.
    echo
    echo '=== journal for every failed unit (this boot) ==='
    systemctl --failed --no-legend --plain --no-pager 2>/dev/null | awk '{print $1}' \
    | while read -r unit; do
        [ -n "$unit" ] || continue
        echo "--- $unit ---"
        journalctl -b --no-pager -o short-precise -u "$unit" | tail -n 60 || true
      done

    # LESSON 2: display-manager.service is an ALIAS. `systemctl status` follows
    # the symlink; `journalctl -u` matches the literal unit a message was logged
    # under, which is always the concrete one. Ask for both.
    dm="$(systemctl show -P Id display-manager.service 2>/dev/null || true)"
    [ -n "$dm" ] || dm=display-manager.service
    echo
    echo "=== display manager ($dm) ==="
    systemctl status "$dm" --no-pager --full || true
    journalctl -b --no-pager -o short-precise -u display-manager.service -u "$dm" \
      | tail -n 80 || true

    # The units this project actually depends on. Named even when they have not
    # failed, because "it is running and doing nothing" is a real outcome for a
    # portal and does not show up above.
    echo
    echo '=== portals and the session bus ==='
    systemctl --no-pager --full list-units 'xdg-desktop-portal*' 'dbus*' || true
    journalctl -b --no-pager -o short-precise \
      -u dbus-broker.service -u dbus.socket | tail -n 40 || true

    # Our own processes are started from the CLI rather than by a unit, so they
    # log to the session journal under the user's uid and appear nowhere above.
    echo
    echo "=== everything $SESSION_USER logged this boot ==="
    journalctl -b --no-pager -o short-precise "_UID=$(uid)" | tail -n 120 || true

    # LESSON 3: a unit that restart-loops or whose dependency was cancelled
    # never reaches "failed", so it leaves nothing in the loop above. This is
    # the backstop that catches it.
    echo
    echo '=== priority<=err, this boot ==='
    journalctl -b --no-pager -o short-precise -p err | tail -n 80 || true

    echo
    echo '=== coredumps ==='
    # Guarded rather than assumed: an image without systemd-coredump would
    # otherwise contribute three "not found" lines and no information.
    if command -v coredumpctl >/dev/null 2>&1; then
      coredumpctl --no-pager --no-legend list 2>&1 | tail -n 30 || true
    else
      echo 'no coredumpctl in this image'
    fi
    ;;

  # WHAT THE LAUNCHER ACTUALLY DID, from its own account rather than from pixels.
  #
  # Every gate in this tier asserts a percentage of changed pixels inside a box.
  # That is the only way to prove something reached the screen, and it is bad at
  # everything else: "5.44% of pixels in a box changed" cannot say which row is
  # selected, what the query matched, or what the panel contains, and it moves
  # with the font, the theme and the card geometry. Two runs of this tier were
  # spent on a containment box that had gone stale and was asserting nothing
  # about the launcher at all.
  #
  # `compass_ui::state` logs one line per message (LauncherApp::state_line), so
  # those claims can be made exactly. What this CANNOT do is replace the pixel
  # gates: the line is written by the same code under test and says nothing
  # about whether anything was drawn -- a launcher rendering a blank surface
  # logs exactly this. That is the failure --require-paint exists for, and the
  # two halves answer different questions.
  #
  # Usage: checks.sh ui-state <key=value>...
  # Each argument must appear on the LAST state line. Without arguments the
  # whole sequence is printed and nothing is asserted.
  # What does cold start cost? (#13 §8.5 -- RECORDED, NOT GATED)
  #
  # §8.5 names "cold start to first frame < 120 ms" and nothing has ever
  # measured it. `LauncherApp` logs `first_draw_ms` when Iced asks for the
  # first frame, and this surfaces it.
  #
  # IT IS A FLOOR ON THE SLA, NOT THE SLA, for two reasons stated here so the
  # number is not quoted as something it is not:
  #
  #   * Iced yields that event on `RedrawRequested` -- the compositor asking
  #     for a frame, not a frame reaching the screen. The rendering after it
  #     is unmeasured, and under llvmpipe it is not small.
  #   * The clock starts inside `compass::run`, so dynamic linking is outside
  #     it, and a binary that links wgpu does not link instantly.
  #
  # Both omissions push the figure DOWN, so a reading over 120 ms would be
  # conclusive while one under it is not. Recorded until there are enough
  # numbers to say what a threshold should be (ADR-0010).
  cold-start)
    if [ ! -s "$UI_ERR" ]; then
      echo "FAIL: $UI_ERR is empty; the launcher logged nothing at all" >&2
      exit 1
    fi
    # Escapes stripped for the reason ui-state strips them: tracing colours
    # its output and a literal pattern matches nothing against it.
    line="$(sed 's/\x1b\[[0-9;]*m//g' "$UI_ERR" | grep -F 'first_draw_ms' | head -1 || true)"
    if [ -z "$line" ]; then
      echo "no first-frame line logged; the launcher drew nothing, or the"
      echo "instrumentation in LauncherApp::update was removed"
      echo "--- tail of the launcher log ---"
      tail -n 20 "$UI_ERR"
      exit 1
    fi
    ms="$(printf '%s' "$line" | sed -n 's/.*first_draw_ms=\([0-9]\+\).*/\1/p')"
    if [ -z "$ms" ]; then
      echo "FAIL: found the line but not the figure, so the field was renamed:" >&2
      printf '  %s\n' "$line" >&2
      exit 1
    fi
    printf 'cold start: first frame requested %s ms after compass::run was entered\n' "$ms"
    printf '  §8.5 names 120 ms to first FRAME. This is a floor on that:\n'
    printf '  it excludes the render after the redraw request, and dynamic\n'
    printf '  linking before the clock starts. Recorded, not gated.\n'
    ;;

  ui-state)
    shift
    if [ ! -s "$UI_ERR" ]; then
      echo "FAIL: $UI_ERR is empty; the launcher logged nothing at all" >&2
      exit 1
    fi

    # The escapes are stripped for the same reason engine-index strips them:
    # tracing colours its output, and a pattern with a literal `query=` matches
    # nothing against `^[[3mquery^[[0m=`. That cost a false negative once
    # already and a grep in a gate should not be the thing that notices.
    sed 's/\x1b\[[0-9;]*m//g' "$UI_ERR" | grep -F 'compass_ui::state' > /tmp/ui-state.log || true

    if [ ! -s /tmp/ui-state.log ]; then
      echo "FAIL: the launcher never logged a state line." >&2
      echo "  Either RUST_LOG no longer enables compass_ui::state=debug, or" >&2
      echo "  LauncherApp::update stopped emitting one." >&2
      echo "--- what it did log ---" >&2
      tail -n 40 "$UI_ERR" >&2
      exit 1
    fi

    echo "--- every state the launcher passed through ---"
    # The whole sequence, in the job output, which is the point: a human
    # reading a failed run sees what the launcher did without downloading an
    # artifact and comparing images.
    sed 's/.*compass_ui::state: //' /tmp/ui-state.log | cat -n
    last="$(tail -n 1 /tmp/ui-state.log)"
    echo "--- asserting against the last line ---"
    echo "  $last"

    status=0
    for want in "$@"; do
      if printf '%s' "$last" | grep -qF -- "$want"; then
        echo "  ok: $want"
      else
        echo "  FAIL: expected $want" >&2
        status=1
      fi
    done
    exit "$status"
    ;;

  harvest-corpus)
    scratch=/tmp/compass-corpus
    rm -rf "$scratch"; mkdir -p "$scratch"
    /usr/libexec/compass-vmtest/harvest-desktop-corpus.sh --out "$scratch" || true

    count=$(find "$scratch" -type f -name '*.desktop' | wc -l)
    echo "harvested $count desktop entries from this image"
    if [ "$count" -eq 0 ]; then
      echo 'no entries harvested — the roots the script knows about are all empty' >&2
      exit 1
    fi

    # Deterministic ordering and no timestamps, so re-running produces the same
    # bytes and a diff of two harvests is a diff of the application set.
    tar --sort=name --mtime='UTC 1970-01-01' --owner=0 --group=0 --numeric-owner \
      -czf "$KBD_CAP_DIR/corpus.tar.gz" -C "$scratch" . 2>/dev/null \
      || tar -czf "$KBD_CAP_DIR/corpus.tar.gz" -C "$scratch" .
    echo "wrote $KBD_CAP_DIR/corpus.tar.gz ($(wc -c < "$KBD_CAP_DIR/corpus.tar.gz") bytes)"

    # A sample in the log, so the artifact is not the only way to see what came
    # out and a wrong-looking harvest is obvious in the run itself.
    echo '--- first 15 entries ---'
    find "$scratch" -type f -name '*.desktop' -printf '  %f\n' | sort | head -15
    ;;

  # Assert the launcher is still up, and say what it printed.
  #
  # Run after the host has screenshotted and typed at it. A launcher that
  # The control this job should have had from the start: can ANY client draw
  # in this session?
  #
  # Everything measured so far says our launcher does not put a window on
  # screen. None of it distinguishes that from *nothing* putting a window on
  # screen — a session where no client can render at all would produce exactly
  # the same evidence, and would exonerate the launcher entirely. The desktop
  # itself painting (deviation 0.1564) does not settle it: that is GNOME Shell
  # compositing its own furniture, not a client surface.
  #
  # So: start a stock GNOME application and let the host screenshot. If it
  # draws and ours does not, the fault is ours. If neither draws, the fault is
  # the session, and every conclusion about the launcher is void.
  #
  # The application is chosen at runtime from what the image actually has,
  # rather than guessed at here: a hard-coded name that is absent reads as
  # "the control failed" when it means "the control never ran".
  control-app-start)
    u="$(uid)"
    : > "$CONTROL_ERR"
    rm -f "$CONTROL_DONE"

    app=""
    for candidate in gnome-text-editor nautilus gnome-calculator ptyxis gnome-terminal gedit; do
      if command -v "$candidate" >/dev/null 2>&1; then app="$candidate"; break; fi
    done
    if [ -z "$app" ]; then
      echo "no stock GNOME application found to use as a control" >&2
      echo "tried: gnome-text-editor nautilus gnome-calculator ptyxis gnome-terminal gedit" >&2
      exit 1
    fi
    echo "control application: $app"

    setsid bash -c '
      runuser -u "$1" -- env \
        XDG_RUNTIME_DIR="/run/user/$2" \
        DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$2/bus" \
        WAYLAND_DISPLAY="$3" \
        XDG_SESSION_TYPE=wayland \
        "$4" > "$5" 2>&1
      echo "$?" > "$6"
    ' _ "$SESSION_USER" "$u" "$(wayland_display)" "$app" \
      "$CONTROL_ERR" "$CONTROL_DONE" \
      < /dev/null >> "$CONTROL_ERR" 2>&1 &

    # NOT pgrep. `-f "$app"` would match the shell evaluating it, because the
    # app's name is in that shell's own command line — the same reflexivity
    # that cost Spike A's collector 180s a run, and which I wrote again here
    # before catching it. `-x` is no escape either: comm is truncated to 15
    # characters, so `gnome-text-editor` is `gnome-text-edit` and an exact
    # match silently never fires.
    #
    # Comparing /proc/PID/exe has neither problem: it is the resolved binary,
    # not a string anyone typed, and it is not truncated. `exe_running` is a
    # function, which works because wait_for invokes "$@" in this same shell.
    wait_for "the control application to appear, or exit" 90 \
      exe_running "$app" "$CONTROL_DONE"

    if [ -f "$CONTROL_DONE" ]; then
      echo "the control application exited $(cat "$CONTROL_DONE"); its output follows"
      cat "$CONTROL_ERR"
    else
      echo "the control application is running"
      cat "$CONTROL_ERR"
    fi
    ;;

  # crashed on the first keystroke is a real bug and would otherwise be visible
  # only as two screenshots that happen to look similar.
  launcher-status)
    if [ -f "$UI_DONE" ]; then
      echo "the launcher exited $(cat "$UI_DONE")" >&2
      cat "$UI_ERR" >&2
      exit 1
    fi
    echo "the launcher is still running"
    cat "$UI_ERR"
    ;;

  # Start the engine, so the launcher has something to attach to.
  #
  # ADR-0015 made the launcher window resident and driven: `compass ui` connects
  # to `compass serve` and waits to be told to show. So the engine has to be up
  # BEFORE launcher-start, or the launcher comes up undriven and every summon
  # below is refused -- correctly, and confusingly.
  #
  # Inside the same Flatpak and the same session as the launcher, because the
  # socket lives under $XDG_RUNTIME_DIR and a daemon in a different runtime dir
  # is a daemon the launcher cannot find.
  #
  # WITH `--no-hotkey`, AND THAT WAS MEASURED RATHER THAN ASSUMED. Binding the
  # GlobalShortcuts portal makes GNOME ask the user for permission, and step 0d
  # measured what that puts on screen: 1.62% of pixels in a 496x532 box. That
  # is correct product behaviour and it is Spike A's job to exercise it -- but
  # inside THIS job it sits between the frames of a gate about the launcher,
  # and moving the control after the engine was not enough, because the
  # launcher window then interacts with it. Two runs failed identically at
  # 962x603 before the flag existed.
  #
  # The flag is not a test hook: a user whose compositor binds a key to
  # `compass toggle` should not be asked to grant one they will not use.
  engine-start)
    u="$(uid)"
    : > "$ENGINE_ERR"
    rm -f "$ENGINE_DONE"

    setsid bash -c '
      runuser -u "$1" -- env \
        XDG_RUNTIME_DIR="/run/user/$2" \
        DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$2/bus" \
        WAYLAND_DISPLAY="$3" \
        XDG_SESSION_TYPE=wayland \
        RUST_LOG=info \
        RUST_BACKTRACE=1 \
        flatpak run --installation="$4" "$5" serve --no-hotkey \
        > "$6" 2>&1
      echo "$?" > "$7"
    ' _ "$SESSION_USER" "$u" "$(wayland_display)" "$INSTALLATION" "$APP" \
      "$ENGINE_ERR" "$ENGINE_DONE" \
      < /dev/null >> "$ENGINE_ERR" 2>&1 &

    # Waited on by asking it, not by looking for its process. A `serve` that is
    # alive but has not yet bound its socket would pass a pgrep and fail every
    # summon after it -- and the engine indexes the machine's applications
    # before it listens, which under llvmpipe is not instant. `ping` answers
    # only once the socket is up.
    # A bare `wait_for` here says only "timed out", which is the same message
    # for an engine that is still indexing, an engine that crashed, and a
    # client invocation that could never have worked. The last of those is
    # exactly what happened once, and the 120 s of silence is what made it
    # expensive. So the timeout now says what `ping` actually returns.
    if ! wait_for "the engine to answer a ping, or exit" 120 engine_ready; then
      echo "--- what the client actually says ---" >&2
      compass_cli ping >&2 2>&1 || true
      echo "--- the engine's own output ---" >&2
      cat "$ENGINE_ERR" >&2
      exit 1
    fi

    # Recorded before the launcher starts, so `launcher_pid` can tell the two
    # apart. Written even on the failure path below: a half-started engine
    # still leaves a process named `compass` around to be mistaken for the
    # launcher.
    pgrep -u "$SESSION_USER" -x compass > "$ENGINE_PIDS" 2>/dev/null || : > "$ENGINE_PIDS"
    echo "engine pids: $(tr '\n' ' ' < "$ENGINE_PIDS")"

    if [ -f "$ENGINE_DONE" ]; then
      echo "the engine exited $(cat "$ENGINE_DONE") instead of listening; its output follows" >&2
      cat "$ENGINE_ERR" >&2
      exit 1
    fi
    echo "the engine is listening; output so far:"
    cat "$ENGINE_ERR"
    ;;

  # Is the launcher window actually attached to the engine?
  #
  # The distinguishing check, and the reason the summon verbs below can mean
  # anything. `serve` refuses show/hide/toggle when no window has attached, so
  # a `toggle` that SUCCEEDS is proof that the whole chain exists: CLI, socket,
  # engine, window link, and a window that answered. A `toggle` that fails with
  # "no launcher window is connected" says precisely which link is missing.
  #
  # This is what the tier could never assert before: every earlier check could
  # only see a process, never a connection.
  window-attached)
    if ! out="$(compass_cli toggle 2>&1)"; then
      echo "the launcher window is not attached to the engine:" >&2
      echo "$out" >&2
      exit 1
    fi
    # Left hidden by the toggle above; `summon` below puts it back. Said out
    # loud because a reader of the log otherwise sees a window vanish for no
    # stated reason.
    echo "the window answered a toggle (and is now hidden): $out"
    ;;

  # Summon the window and report how long the round trip took.
  #
  # §8.5's "summon to first frame" row. This is NOT that number: nothing inside
  # the guest can observe a frame (ADR-0010), so what is timed here is the
  # round trip -- CLI to engine, engine to window, window's answer back. First
  # paint follows it.
  #
  # It is also inflated by a whole process spawn, because the client is
  # `compass toggle` rather than a keypress. On the real path the engine is
  # already running and the portal delivers the activation directly, so this
  # number is an upper bound with a Flatpak launch inside it. Reported, not
  # gated, for the reasons §8.5 gives.
  summon)
    start_ms="$(date +%s%3N)"
    if ! out="$(compass_cli show 2>&1)"; then
      echo "the engine could not show the window:" >&2
      echo "$out" >&2
      exit 1
    fi
    end_ms="$(date +%s%3N)"
    echo "summon round trip: $((end_ms - start_ms)) ms \
(client spawn included; not a frame -- see §8.5)"
    ;;

  # Hide it again, so the host can screenshot the difference.
  dismiss)
    compass_cli hide
    ;;

  # What did the engine say about the hotkey?
  #
  # Recorded, not gated. Whether GNOME grants LOGO+space is the user's decision
  # via a permission dialog, and an unattended session may well be refused --
  # which is a real outcome worth seeing in the log, not a failure of the code.
  hotkey-status)
    grep -E "launcher hotkey|GlobalShortcuts|shortcut" "$ENGINE_ERR" || \
      echo "the engine said nothing about the hotkey"
    ;;

  # Put the session on a bare desktop before anything is measured against it.
  #
  # GNOME Shell opens the Activities overview at login when the session has no
  # windows, and until now the session always had one: the first-run tour. With
  # the tour suppressed the overview is what the tier's "bare desktop" frame
  # actually shows, and the first run after that suppression shows exactly what
  # that costs -- the launcher opened BEHIND the overview and was captured as a
  # scaled thumbnail inside a workspace tile, so the gate that says "a launcher
  # window appeared" was measuring a preview of one.
  #
  # It is dismissed over the session bus rather than by injecting Escape, and
  # the difference is the point: `org.gnome.Shell.OverviewActive` is a readwrite
  # boolean, so this says what it wants and then READS BACK whether it got it. A
  # keystroke can only be sent, and a tier that cannot tell "the overview closed"
  # from "the key went nowhere" is how the wrong conclusion below got written
  # down in the first place.
  #
  # Idempotent by construction: setting it false when it is already false is
  # fine, and the read-back is then trivially true. So this stays correct if a
  # later GNOME stops opening the overview at login, rather than becoming a step
  # that fails because it had nothing to do.
  overview-dismiss)
    u="$(uid)"
    shell_prop() {
      local method="$1"; shift
      runuser -u "$SESSION_USER" -- env \
        XDG_RUNTIME_DIR="/run/user/$u" \
        DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$u/bus" \
        gdbus call --session \
          --dest org.gnome.Shell --object-path /org/gnome/Shell \
          --method "org.freedesktop.DBus.Properties.$method" \
          org.gnome.Shell OverviewActive "$@"
    }
    overview_closed() { case "$(shell_prop Get 2>/dev/null)" in *false*) return 0 ;; esac; return 1; }

    echo "OverviewActive at login: $(shell_prop Get 2>&1 || echo '(unreadable)')"

    if ! shell_prop Set "<false>" >/dev/null 2>&1; then
      echo "could not set org.gnome.Shell OverviewActive -- is gnome-shell on the session bus?" >&2
      shell_prop Get >&2 2>&1 || true
      exit 1
    fi

    # Read back, rather than trusting the write. The property is set on the
    # shell's side of an async animation, so the value can be false while the
    # overview is still on its way out; the poll covers the first and the sleep
    # covers the second. If the frame is still mid-animation the run does not
    # silently pass on it -- step 0d compares this desktop against the one taken
    # after the engine starts and requires them identical, which a moving
    # overview cannot be.
    if ! wait_for "the overview to report itself closed" 30 overview_closed; then
      echo "OverviewActive is still: $(shell_prop Get 2>&1 || echo '(unreadable)')" >&2
      exit 1
    fi
    sleep 2
    echo "OverviewActive now: $(shell_prop Get 2>&1 || echo '(unreadable)')"
    ;;

  # Did the engine actually find any applications? (#95)
  #
  # THE CHEAPEST GATE IN THIS FILE, AND IT WOULD HAVE SAVED MONTHS.
  #
  # `indexed applications applications=0` was printed by every run of this tier and read by
  # nobody, because an empty index and a working one looked identical on screen: until the search
  # field could be focused at all, both drew "Type to search..." forever. The launcher shipped
  # unable to see a single application on the machine it was running on.
  #
  # A missing line fails as loudly as a zero. "The engine did not say" and "the engine said none"
  # are both answers this check must not treat as success -- a log format change that silently
  # turned this into a no-op is exactly the failure mode the gate exists to prevent.
  engine-index)
    # THE ESCAPES ARE NOT DECORATION, THEY ARE WHY THIS FAILED THE FIRST TIME IT RAN.
    #
    # `tracing_subscriber::fmt` colours its output whether or not the writer is a terminal, so
    # the engine's log file holds
    #
    #     indexed applications ^[[3mapplications^[[0m^[[2m=^[[0m15
    #
    # and a pattern with a literal `applications=` matches nothing. The gate then reported "the
    # engine never reported an application count" while printing the line that contained it --
    # a false negative that read exactly like the real failure it exists to catch.
    #
    # The engine no longer colours a non-terminal, so this is belt and braces. It stays because
    # RUST_LOG and a future writer can both put the escapes back, and a grep in a gate should not
    # be the thing that notices.
    line="$(sed 's/\x1b\[[0-9;]*m//g' "$ENGINE_ERR" \
            | grep -o 'indexed applications applications=[0-9]*' | tail -1 || true)"
    if [ -z "$line" ]; then
      echo "the engine never reported an application count; the log format has probably changed" >&2
      echo "--- what it did say ---" >&2
      cat "$ENGINE_ERR" >&2
      exit 1
    fi

    count="${line##*=}"
    echo "the engine indexed $count applications"

    # THE FLOOR IS DERIVED FROM A MEASUREMENT, not from "more than nothing".
    #
    # This asserted `-gt 0` because nobody had a real number. #97 produced one by
    # reconciling the image's 88 entries against what the engine reports:
    #
    #   88 entries on disk
    #   -70 NoDisplay=true        <- almost all of the filtering
    #    -3 Hidden=true
    #   ---
    #    11 visible with no desktop identified
    #    +4 desktop actions on those
    #   ---
    #    15   exactly what the engine reported
    #
    # So 15 is the floor's evidence, and 10 is the floor: well under the measured value, well
    # over zero. A floor rather than an equality because the image's application set moves when
    # Bluefin rebases, and an equality would turn somebody else's release into our red run.
    #
    # The number should now be HIGHER than 15, not lower: #97's fallback lets the session be
    # identified, which admits the OnlyShowIn=GNOME entries that were being hidden on GNOME (one
    # on this image, more on others). That is why the floor is not set at 15 -- it is set below
    # the number this gate was calibrated against, so the fix cannot trip it either way.
    INDEX_FLOOR=10
    if [ "$count" -lt "$INDEX_FLOOR" ]; then
      if [ "$count" -eq 0 ]; then
        echo "the engine indexed ZERO applications: the launcher can see nothing to launch (#95)" >&2
      else
        echo "the engine indexed only $count applications, under the floor of $INDEX_FLOOR" >&2
        echo "  #97 reconciled this image to 15 (11 visible entries + 4 desktop actions)." >&2
        echo "  A count this low means entries are being filtered that should not be -- check" >&2
        echo "  the sandbox's application dirs and whether the desktop is being identified." >&2
      fi
      echo "--- where the doctor looked ---" >&2
      compass_cli doctor 2>&1 | grep -A 20 'xdg.application-dirs' >&2 || true
      exit 1
    fi
    ;;

  *)
    echo "unknown subcommand: $1" >&2
    exit 64
    ;;
esac
