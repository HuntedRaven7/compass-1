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
SPIKE_OUT=/tmp/compass-spike-a.json
SPIKE_ERR=/tmp/compass-spike-a.err
SPIKE_DONE=/tmp/compass-spike-a.done
UI_ERR=/tmp/compass-ui.err
UI_DONE=/tmp/compass-ui.done

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
        RUST_LOG="info,wgpu=debug,wgpu_hal=debug,iced_wgpu=debug,winit=debug,\
sctk_adwaita=debug,smithay_client_toolkit=debug,wayland_client=debug,calloop=debug" \
        RUST_BACKTRACE=1 \
        flatpak run --installation="$4" "$5" ui \
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
    wait_for "the launcher process to appear, or exit" 90 \
      bash -c 'pgrep -u "$1" -x vicinae >/dev/null || [ -f "$2" ]' \
      _ "$SESSION_USER" "$UI_DONE"

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
    pid="$(pgrep -u "$SESSION_USER" -x vicinae | head -1 || true)"
    if [ -z "$pid" ]; then
      echo "no vicinae process to diagnose"
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
    # Matching the socket inodes against unix sockets gives the peer path, which
    # is what distinguishes "waiting on Wayland" from "waiting on the portal".
    ss -x -p 2>/dev/null | grep -F "pid=$pid" || echo '(ss unavailable or no match)'
    ;;

  # Assert the launcher is still up, and say what it printed.
  #
  # Run after the host has screenshotted and typed at it. A launcher that
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

  *)
    echo "unknown subcommand: $1" >&2
    exit 64
    ;;
esac
