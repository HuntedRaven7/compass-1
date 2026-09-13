#!/usr/bin/env bash
# Drive the launcher: open it in the guest, look at the screen, type at it.
#
# Usage: scripts/vmtest/launcher.sh <artifact-dir> <vm-name>
#
# WHY THIS IS NOT A `--check`
#
# Same reason as spike-a.sh. The question here is "did a launcher appear on
# screen", and the only observer of the framebuffer is corral, on the host side
# of QEMU. Nothing inside the guest can answer it — ADR-0010 says so and it is
# still true. So the sequence straddles guest and host, which means running
# after `corral vmtest` returns, against a VM it was told to leave up.
#
# WHAT IS GATED, AND WHAT IS ONLY RECORDED
#
# Gated: the launcher process starts and stays up (checks.sh, in the guest),
# and the screen still passes corral's own blank test with the launcher open.
# That threshold is corral's, not one invented here.
#
# Recorded, not gated: the three luminance deviations. It is tempting to assert
# that opening a launcher raises the deviation, and it probably does — but this
# has never been measured once, and ADR-0010 is explicit that inventing a pixel
# threshold before seeing real numbers is how a tier starts flaking. Spike A and
# Spike B both shipped gating on nothing but "produced a report", for the same
# reason. Once a few runs have published numbers, the gate can be set from data
# rather than from a guess, and that is a two-line change to this file.
set -euo pipefail

out="${1:?usage: launcher.sh <artifact-dir> <vm-name>}"
vm="${2:?usage: launcher.sh <artifact-dir> <vm-name>}"
query="${LAUNCHER_QUERY:-fi}"
checks=/usr/libexec/compass-vmtest/checks.sh

# corral wrote result.json as root and it holds the run's own SSH details.
# Reading them beats rebuilding the invocation here and drifting out of step.
sudo chown -R "$(id -un)" "$out" 2>/dev/null || true
mapfile -t ssh_argv < <(RESULT="$out/result.json" python3 - <<'PY'
import json, os
ssh = json.load(open(os.environ['RESULT']))['ssh']
for part in ['ssh',
             '-i', ssh['identityFile'],
             '-p', str(ssh['port']),
             '-o', 'StrictHostKeyChecking=no',
             '-o', 'UserKnownHostsFile=/dev/null',
             '-o', 'LogLevel=ERROR',
             '-o', 'BatchMode=yes',
             f"{ssh['user']}@{ssh['host']}"]:
    print(part)
PY
)

guest() { "${ssh_argv[@]}" "$@"; }

# `sudo -E "$(command -v corral)"`, not `sudo corral`: corral is on PATH via
# GITHUB_PATH, and sudo replaces PATH with secure_path, so `sudo corral` is
# "command not found". Verified that -E alone does not help — secure_path wins.
corral_bin="$(command -v corral)"
shot() { sudo -E "$corral_bin" screenshot "$vm" -o "$out/$1" ; }

echo "=== 0. the desktop before the launcher opens (the control) ==="
# Without this frame there is nothing to compare against, and "the launcher is
# on screen" cannot be distinguished from "the desktop was always like that".
shot "launcher-00-before.png"

echo
echo "=== 1. open the launcher in the session ==="
guest "$checks" launcher-start

echo
echo "=== 1b. what is the launcher blocked on? ==="
# Added after the instrumented run: the log stops 200ms in and the process sits
# there alive, so logging cannot say where it stopped — the stuck code is not
# the code doing the logging. The kernel can. Best-effort; a diagnostic must
# never fail the run it exists to explain.
guest "$checks" launcher-diagnose || true

echo
echo "=== 2. the screen with the launcher open ==="
sudo -E "$corral_bin" screenshot "$vm" -o "$out/launcher-01-open.png" --require-paint

echo
echo "=== 3. type a query at it ==="
# At QEMU's emulated keyboard, not synthesised inside the session: a keystroke
# the guest sends itself would prove only that our own event loop can talk to
# itself. This is the same mechanism Spike A uses to press Super+Space.
sudo -E "$corral_bin" type "$vm" "$query"
shot "launcher-02-typed.png"

echo
echo "=== 3b. and is it blocked in the same place after the keystroke? ==="
# The same probe again, deliberately. A process parked in the same syscall on
# the same socket both times is stuck; one that has moved is merely slow, and
# those two want completely different fixes.
guest "$checks" launcher-diagnose || true

echo
echo "=== 4. is it still running? ==="
# A launcher that dies on the first keystroke is a real bug, and without this
# it would show up only as two screenshots that happen to look alike.
guest "$checks" launcher-status
