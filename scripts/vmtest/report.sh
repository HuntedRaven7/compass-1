#!/usr/bin/env bash
# Say what a `corral vmtest` run reported, in the order a reader needs it.
#
# Usage: scripts/vmtest/report.sh <artifact-dir>
#
# Ordering matters and it was got wrong first time round: the serial tail is
# expanded by ANSI into hundreds of log lines, so a verdict printed before it
# scrolls off the end, and three separate attempts to read an exit code came
# back showing only the boot. The verdict therefore prints LAST and survives any
# amount of truncation.
set -uo pipefail

out="${1:?usage: report.sh <artifact-dir>}"

# corral ran under sudo, so everything it wrote is root-owned and
# upload-artifact cannot even scandir it (EACCES on <out>/ssh).
sudo chown -R "$(id -un)" "$out" 2>/dev/null || true

if [ ! -f "$out/result.json" ]; then
  echo "no result.json — the run died before writing one"
else
  # Deliberately not the whole document: the frame list is ~20 entries and
  # buries the fields that decide what to do next.
  RESULT="$out/result.json" python3 - <<'PY'
import json, os
r = json.load(open(os.environ['RESULT']))
frames = r.pop('frames', [])
checks = r.pop('checks', [])
print(json.dumps(r, indent=2)[:3000])
if frames:
    devs = [f.get('stddev', 0) for f in frames]
    print(f"\nframes: {len(frames)}  stddev min={min(devs):.4f} max={max(devs):.4f}"
          f"  (corral calls <=0.02 blank)")
if checks:
    print(f"\n--- {len(checks)} checks ---")
    for c in checks:
        print(f"\n[{'PASS' if c.get('passed') else 'FAIL'}] {c.get('command')}")
        if c.get('output'):
            print('\n'.join('    ' + line for line in c['output'].splitlines()[:60]))
        if c.get('error'):
            print(f"    error: {c['error']}")
PY
fi

# The targets the guest actually reached. This is the decisive diagnostic for a
# readiness timeout: the first ever run matched nothing because the marker was
# 'Reached target Graphical' while systemd prints the unit name lowercase
# ("Reached target graphical.target - Graphical Interface."). Printing the real
# lines means a second timeout is answered by reading, not by guessing again.
echo '--- targets reached, and the display-manager path ---'
if [ -f "$out/serial.log" ]; then
  # Strip ANSI first. systemd colourises the unit name, so the escape sequence
  # sits between "target " and "network.target" and an un-stripped grep silently
  # reports only the early-boot lines — which is exactly how an earlier run's
  # diagnostic misled me.
  sed -e 's/\x1b\[[0-9;?]*[a-zA-Z]//g' -e 's/\x1b\][^\x07\x1b]*(\x07|\x1b\\\\)//g' \
    "$out/serial.log" > /tmp/serial-plain.log 2>/dev/null || cp "$out/serial.log" /tmp/serial-plain.log
  grep -aoE 'Reached target [A-Za-z0-9._@-]+' /tmp/serial-plain.log | sort -u || true
  echo '--- gdm / plymouth / display ---'
  grep -aiE 'COMPASS-VMTEST|gdm|plymouth|graphical|wayland|greeter' /tmp/serial-plain.log | tail -40 || true
else
  echo "no serial.log"
fi

echo '--- serial console tail ---'
tail -40 "$out/serial.log" 2>/dev/null || echo "no serial.log"

echo '=============================================================='
if [ -f "$out/result.json" ]; then
  RESULT="$out/result.json" python3 - <<'PY'
import json, os
r = json.load(open(os.environ['RESULT']))
print('VERDICT:', r.get('status'), '| exit', r.get('exitCode'),
      '| ready', r.get('ready'), '| bootSeconds', r.get('bootSeconds'))
failed = [c['command'] for c in r.get('checks', []) if not c.get('passed')]
if failed:
    print('CHECKS FAILED:', len(failed))
    for c in failed:
        print('  -', c)
if r.get('failure'):
    print('FAILURE:', str(r['failure']).splitlines()[-1][:400])
PY
else
  echo 'VERDICT: no result.json'
fi
echo '=============================================================='
