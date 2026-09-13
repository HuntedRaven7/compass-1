#!/usr/bin/env bash
# Drive Spike A: bind a global shortcut in the guest, then press the key from
# outside it.
#
# Usage: scripts/vmtest/spike-a.sh <artifact-dir> <vm-name>
#
# WHY THIS IS NOT A `--check`
#
# corral runs each --check over its own SSH connection, and the whole point of
# Spike A is a keypress that does NOT come from inside the session under test.
# A hotkey synthesised in the guest would prove only that our own code can send
# itself an event; a scancode at QEMU's emulated keyboard is indistinguishable
# from a person pressing the key. That injection is a host command, so the
# sequence has to straddle the guest and the host — which means running after
# `corral vmtest` returns, against a VM it was told to leave up.
#
# WHAT COUNTS AS SUCCESS
#
# The spike producing a report. Nothing else. Every outcome it can record is an
# answer to the question #3 asks — including "GNOME will not bind without a
# dialog nobody in CI can click", which is a finding worth having and not a
# broken run. Gating on `activated: true` would turn that answer into a red X
# with no information in it.
set -euo pipefail

out="${1:?usage: spike-a.sh <artifact-dir> <vm-name>}"
vm="${2:?usage: spike-a.sh <artifact-dir> <vm-name>}"
trigger="${SPIKE_A_TRIGGER:-SUPER+space}"
checks=/usr/libexec/compass-vmtest/checks.sh

# corral wrote result.json as root and it holds the run's own SSH details —
# port, key path, user. Reading them is better than rebuilding the invocation
# here and getting quietly out of step with corral.
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

echo "=== 0. the state the spike is about to run against ==="
# Before the spike, not after: if the bind hangs, the run's whole value is in
# knowing whether the pre-seeded grant was actually visible to the session, and
# a step that runs afterwards would be waiting behind the hang to say so.
# Best-effort — evidence must never be what fails the run.
guest "$checks" spike-a-evidence || true

echo
echo "=== 1. bind the shortcut in the guest ==="
guest "$checks" spike-a-start "$trigger"

echo
echo '=== 1b. what is on screen at the moment we press ==='
# The direct test of why a bind might not complete. If GNOME is showing a
# consent dialog, it is in this frame, and nothing else distinguishes "the
# portal is waiting for a human" from "the portal is broken". Best-effort: a
# spike must not lose its answer because a screenshot failed.
sudo -E "$(command -v corral)" screenshot "$vm" -o "$out/at-keypress.png" || true

echo
echo "=== 2. press it at QEMU's emulated keyboard ==="
# meta_l and spc are QEMU key names, passed through unvalidated by corral, and
# `key` presses them together. This is the step the issue flagged as expected to
# work but never run.
# `sudo -E "$(command -v corral)"`, not `sudo corral`. corral is on PATH via
# GITHUB_PATH, and sudo replaces PATH with secure_path — so `sudo corral` is
# "command not found" while every other step in this workflow, which resolves
# the absolute path first, works. That is exactly how this failed the first time.
sudo -E "$(command -v corral)" key "$vm" meta_l spc
echo "sent: meta_l spc"

echo
echo "=== 3. what the desktop did with it ==="
guest "$checks" spike-a-collect | tee "$out/spike-a.txt"

# Split the report back out as its own artifact, so the answer is a file rather
# than something to be recovered from a log.
if sed -n '/^--- report ---$/,$p' "$out/spike-a.txt" | tail -n +2 > "$out/spike-a.json"; then
  python3 -c "import json,sys; json.load(open('$out/spike-a.json'))" 2>/dev/null \
    || rm -f "$out/spike-a.json"
fi

echo
echo '=============================================================='
if [ -f "$out/spike-a.json" ]; then
  REPORT="$out/spike-a.json" python3 - <<'PY'
import json, os
r = json.load(open(os.environ['REPORT']))
print('SPIKE A: portal', r['portal'], '| bind', r['bind_outcome'],
      '| activated', r['activated'])
if r.get('bound'):
    for b in r['bound']:
        print('  bound', b['id'], '->', b['trigger_description'] or '(no trigger reported)')
match = r.get('trigger_matches_request')
if match is not None:
    print('  granted the requested trigger:', 'yes' if match else 'NO')
if r.get('activation_token_present') is not None:
    print('  xdg-activation token:', 'present' if r['activation_token_present'] else 'absent')
for note in r.get('notes', []):
    print('  note:', note)
PY
else
  echo 'SPIKE A: no report — see the output above'
fi
echo '=============================================================='
