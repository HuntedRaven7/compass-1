#!/usr/bin/env python3
"""Controls for headless-gnome-spike.sh's readiness waits (#333).

The spike's waits had deadlines, but a deadline is only checked between
attempts: one `wayland-info` that connected and was never answered held the
loop for as long as it liked. And a compositor that died mid-wait was reported
as a timeout a minute later rather than as the death it was.

This runs the real spike against stub `mutter`, `wayland-info` and
`dbus-launch`, each staged to fail one way, and asserts that the spike:

    passes against a compositor that serves the registry           (control)
    stops a hung wayland-info inside its probe budget and gives up
      at the startup deadline, printing the last attempt
    reports a compositor that exits mid-wait as an exit, at once
    stops a hung dbus-launch inside its budget
    passes the command-under-test's exit status through

and that on every one of those paths the compositor's whole process group,
the stub bus daemon and any hung probe's children are gone afterwards, while an
unrelated sentinel process is not.

Needs no compositor: everything the spike runs is a stub. Run it directly; it
exits non-zero if any control behaves wrongly.
"""

from __future__ import annotations

import os
import re
import shutil
import subprocess
import tempfile
import time
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
SPIKE = HERE / "headless-gnome-spike.sh"

SLACK_S = 5

MUTTER = r"""#!/usr/bin/env bash
if [ "${1:-}" = --version ]; then echo "mutter 48.0 (stub)"; exit 0; fi
echo $$ > "$STUB_DIR/mutter-pid"
# A child, so the test can see the compositor's group was stopped and not only
# the compositor.
sleep 600 &
echo $! > "$STUB_DIR/mutter-child"
sleep 0.5
: > "$XDG_RUNTIME_DIR/wayland-0"
echo "stub mutter: listening"
if [ "$STUB_MUTTER" = dies ]; then
  sleep 2
  echo "stub mutter: exiting, as the scenario asks"
  exit 3
fi
wait
"""

WAYLAND_INFO = r"""#!/usr/bin/env bash
case "$STUB_WAYLAND_INFO" in
  ok)
    for i in wl_compositor wl_seat xdg_wm_base xdg_activation_v1; do
      echo "interface: '$i', version: 1, name: 1"
    done ;;
  refuse)
    echo "failed to connect to the display" >&2
    exit 1 ;;
  hang)
    sleep 600 &
    echo "$$ $! $(date +%s.%N) wayland-info" >> "$STUB_DIR/hung-pids"
    wait ;;
esac
"""

DBUS_LAUNCH = r"""#!/usr/bin/env bash
if [ "$STUB_DBUS" = hang ]; then
  sleep 600 &
  echo "$$ $! $(date +%s.%N) dbus-launch" >> "$STUB_DIR/hung-pids"
  wait
fi
# A stand-in for the bus daemon, detached the way the real one is.
setsid sleep 600 </dev/null >/dev/null 2>&1 &
echo $! > "$STUB_DIR/bus-pid"
echo "DBUS_SESSION_BUS_ADDRESS='unix:path=$STUB_DIR/bus';"
echo "export DBUS_SESSION_BUS_ADDRESS;"
echo "DBUS_SESSION_BUS_PID=$!;"
"""


def alive(pid: int) -> bool:
    """True for a process that exists and is not a zombie."""
    try:
        state = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[0]
    except (FileNotFoundError, ProcessLookupError, IndexError):
        return False
    return state != "Z"


class Spike(unittest.TestCase):
    def setUp(self) -> None:
        self.dir = Path(tempfile.mkdtemp(prefix="spike-selftest-"))
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)
        self.bin = self.dir / "bin"
        self.bin.mkdir()
        for name, body in (("mutter", MUTTER), ("wayland-info", WAYLAND_INFO),
                           ("dbus-launch", DBUS_LAUNCH), ("gdbus", "#!/bin/sh\nexit 0\n")):
            path = self.bin / name
            path.write_text(body)
            path.chmod(0o755)
        self.sentinel = subprocess.Popen(["sleep", "900"])
        self.addCleanup(self.sentinel.wait)
        self.addCleanup(self.sentinel.kill)

    def spike(self, *args: str, mutter="ok", wayland_info="ok", dbus="ok", **env: str):
        runtime = self.dir / "runtime"
        environ = dict(os.environ)
        environ.update({
            "PATH": f"{self.bin}{os.pathsep}{os.environ['PATH']}",
            "STUB_DIR": str(self.dir),
            "STUB_MUTTER": mutter,
            "STUB_WAYLAND_INFO": wayland_info,
            "STUB_DBUS": dbus,
            "XDG_RUNTIME_DIR": str(runtime),
            "SHELL_LOG": str(self.dir / "mutter.log"),
            "WAYLAND_INFO_LOG": str(self.dir / "wayland-info.txt"),
            "KILL_GRACE_S": "1",
        })
        environ.update(env)
        started = time.time()
        done = subprocess.run(["bash", str(SPIKE), *args], env=environ, capture_output=True,
                              text=True, timeout=300)
        self.elapsed = time.time() - started
        self.output = done.stdout + done.stderr
        return done.returncode

    def report(self) -> str:
        return f"--- spike output ({self.elapsed:.1f}s) ---\n{self.output}"

    def pid(self, name: str) -> int | None:
        path = self.dir / name
        return int(path.read_text().split()[0]) if path.exists() else None

    def assertOwnedStopped(self) -> None:  # noqa: N802 - unittest's naming
        """What the run started is gone; what it did not start is not."""
        owned = [self.pid(n) for n in ("mutter-pid", "mutter-child", "bus-pid")]
        hung = self.dir / "hung-pids"
        if hung.exists():
            for line in hung.read_text().splitlines():
                owned += [int(p) for p in line.split()[:2]]
        owned = [p for p in owned if p is not None]
        deadline = time.time() + 5
        while time.time() < deadline and any(alive(p) for p in owned):
            time.sleep(0.1)
        for pid in owned:
            self.assertFalse(alive(pid), f"process {pid} outlived the spike\n{self.report()}")
        self.assertTrue(alive(self.sentinel.pid), "an unrelated process was stopped")

    def test_a_serving_compositor_passes(self):
        self.assertEqual(self.spike(), 0, self.report())
        self.assertIn("ok: wayland-info to connect", self.output)
        self.assertIn("SPIKE PASSED", self.output)
        self.assertOwnedStopped()

    def test_a_hung_client_probe_is_stopped_and_the_deadline_holds(self):
        status = self.spike(wayland_info="hang", PROBE_TIMEOUT_S="2", STARTUP_TIMEOUT_S="7")
        self.assertEqual(status, 1, self.report())
        # The last attempt may be given less than the probe budget: what is left
        # of the deadline, which is the point of capping it.
        budgets = [int(b) for b in re.findall(
            r"TIMEOUT: `wayland-info` did not finish within its (\d+)s budget", self.output)]
        self.assertTrue(budgets, self.report())
        self.assertTrue(all(1 <= b <= 2 for b in budgets), self.report())
        self.assertIn("SPIKE FAILED: timed out after 7s waiting for: wayland-info to connect",
                      self.output)
        self.assertIn("--- the last attempt (exit 124", self.output)
        attempts = (self.dir / "hung-pids").read_text().splitlines()
        self.assertGreater(len(attempts), 1, "the wait made one attempt\n" + self.report())
        self.assertLess(self.elapsed, 7 + 1 + SLACK_S, self.report())
        self.assertOwnedStopped()

    def test_a_compositor_that_exits_is_reported_as_an_exit_at_once(self):
        status = self.spike(mutter="dies", wayland_info="refuse", STARTUP_TIMEOUT_S="60")
        self.assertEqual(status, 1, self.report())
        self.assertIn("SPIKE FAILED: mutter exited while waiting for: wayland-info to connect",
                      self.output)
        self.assertIn("failed to connect to the display", self.output)
        self.assertLess(self.elapsed, 15, "it waited out the deadline\n" + self.report())
        self.assertOwnedStopped()

    def test_a_hung_dbus_launch_is_stopped(self):
        status = self.spike(dbus="hang", PROBE_TIMEOUT_S="2")
        self.assertEqual(status, 1, self.report())
        self.assertIn("SPIKE FAILED: dbus-launch did not start a session bus within 2s",
                      self.output)
        self.assertLess(self.elapsed, 2 + 1 + SLACK_S, self.report())
        self.assertOwnedStopped()

    def test_the_command_under_test_decides_the_exit_status(self):
        status = self.spike("bash", "-c", 'test -n "$WAYLAND_DISPLAY" && exit 7')
        self.assertEqual(status, 7, self.report())
        self.assertIn("WAYLAND_DISPLAY=wayland-0", self.output)
        self.assertOwnedStopped()


if __name__ == "__main__":
    unittest.main(verbosity=2)
