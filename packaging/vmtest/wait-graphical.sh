#!/usr/bin/env bash
# Say on the serial console when a graphical session actually exists.
#
# This is the VM tier's readiness marker, and it exists because the obvious one
# is wrong. `Started gdm.service` fires the moment systemd starts the unit —
# which is also the moment GDM takes the DRM device and blanks it, several
# seconds before the greeter composites anything. corral captures its final
# frame at readiness and does not retry, so gating on that marker means
# `--require-paint` samples a deliberately blank screen and the run fails with
# "the guest booted but never painted anything" at a framebuffer deviation of
# exactly 0.0000. It is a race, and a run that passes it passed by luck.
#
# So readiness is keyed off a *state* — logind reports a session whose type is
# wayland or x11 — rather than off a unit starting. ADR-0010's rule.
#
# The settle after that state is the one duration here, and it is deliberate: a
# session registers with logind before its compositor draws a first frame, and
# nothing inside the guest can observe "has painted" — the only observer of the
# framebuffer is corral, on the other side of QEMU, and it exposes no wait. Ten
# seconds of llvmpipe is generous for that gap. It is slack after a state, not
# an assertion phrased as a duration.
set -uo pipefail

MARKER='COMPASS-VMTEST: graphical session up'
SETTLE="${COMPASS_VMTEST_SETTLE:-10}"
TIMEOUT="${COMPASS_VMTEST_TIMEOUT:-600}"

# Straight to the console: this has to appear on the serial log corral is
# watching, and journal output does not.
#
# Both /dev/console and /dev/ttyS0, because the whole run hangs for its full
# 30-minute timeout if the marker never reaches the serial log — and which of
# the two is the serial port depends on the kernel's console= arguments, which
# are bootc's to choose, not ours. Writing to both costs a duplicate line in the
# log and removes the failure mode entirely.
say() {
  local line="$*" wrote=0
  local sink
  for sink in /dev/console /dev/ttyS0; do
    [ -w "$sink" ] || continue
    printf '%s\n' "$line" > "$sink" 2>/dev/null && wrote=1
  done
  [ "$wrote" = 1 ] || printf '%s\n' "$line"
}

# True when any logind session is a graphical one. `loginctl list-sessions`
# columns have moved between systemd versions, so the id is read positionally
# and everything else is asked for by name.
graphical_session() {
  local id type
  while read -r id _rest; do
    [ -n "$id" ] || continue
    type="$(loginctl show-session "$id" -p Type --value 2>/dev/null || true)"
    case "$type" in
      wayland | x11) return 0 ;;
    esac
  done < <(loginctl list-sessions --no-legend 2>/dev/null)
  return 1
}

deadline=$((SECONDS + TIMEOUT))
until graphical_session; do
  if ((SECONDS >= deadline)); then
    say "COMPASS-VMTEST: no graphical session after ${TIMEOUT}s"
    # Evidence on the console, where a failed run can still be read: corral
    # copies the serial log whether the guest became ready or not.
    say "$(loginctl list-sessions --no-legend 2>&1 || true)"
    say "$(systemctl --failed --no-legend --no-pager 2>&1 || true)"
    exit 1
  fi
  sleep 2
done

say "COMPASS-VMTEST: graphical session after ${SECONDS}s, settling ${SETTLE}s"
sleep "$SETTLE"

# Printed BEFORE the marker, deliberately. Everything corral does over SSH —
# the checks and the diagnostics bundle — happens after readiness, so when SSH
# does not answer there is no way to ask the guest anything: the only channel
# left is the console, and corral's last copy of the serial log is taken before
# it waits for SSH. So the answer has to already be in the log by the time the
# marker appears.
say "COMPASS-VMTEST: sshd enabled=$(systemctl is-enabled sshd.service 2>&1) active=$(systemctl is-active sshd.service 2>&1)"
say "COMPASS-VMTEST: listening on 22: $(ss -Htln 'sport = :22' 2>&1 | tr '\n' ';')"

say "$MARKER"
