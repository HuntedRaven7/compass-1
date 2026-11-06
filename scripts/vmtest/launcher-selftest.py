#!/usr/bin/env python3
"""Controls for launcher.sh's budgets and transition waits (#333).

WHY THIS FILE EXISTS

launcher.sh runs against a VM that takes twenty-five minutes to boot, so the
paths that matter most when it goes wrong -- a guest that stops answering, a
probe that hangs, a collector that hangs on the way out, a screen that freezes
on the wrong picture -- are exactly the paths a real run almost never takes and
nobody gets to watch. A budget that has never been seen to fire is a guess.

So the real launcher.sh is run here, unchanged, against fakevm.py playing both
the guest (as `ssh`) and corral, with each failure staged on purpose:

    a paint that arrives after the old fixed five-second pause   must pass
    a frozen error screen after hide                             must fail on paint
    a launcher that never says it is hidden                      must fail on state
    a launcher that exits while being waited for                 must fail at once
    a guest that stops answering ssh                             must fail as ssh
    a hung remote command, probe, screenshot, collector          must be stopped
    a real ssh against a server that never sends a banner        must time out
    a run cancelled with SIGTERM mid-call                        must reap the call
    a shell that finishes starting after the boot marker, and
      a desktop still animating when it does (#345)              must pass, on still frames
    a shell that never finishes starting                         must fail before any frame
    a desktop that never holds still                             must fail on stillness

and every hung stub leaves a grandchild behind, which must be gone afterwards,
while an unrelated sentinel process started by this file must still be alive.

Scenarios run concurrently, because each one runs framediff.py on full frames
and serially they take minutes. Budgets are shrunk through launcher.sh's own
VMTEST_* variables; the gates and their thresholds are not touched.

Run it directly; it exits non-zero if any control behaves wrongly.
"""

from __future__ import annotations

import concurrent.futures
import json
import os
import re
import shutil
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import zlib
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
LAUNCHER = HERE / "launcher.sh"
FAKEVM = HERE / "fakevm.py"

W, H = 1280, 800
DESKTOP = (20, 20, 30)

# Each frame is the desktop plus rectangles. The launcher's window sits inside
# the containment box launcher.sh derives from design.rs (x 236..1044 for the
# 768-wide window), so the real gates pass on the real reason.
WINDOW = (280, 160, 999, 700, (200, 200, 210))
QUERY = (320, 200, 520, 220, (255, 255, 255))
PANEL = (600, 300, 900, 500, (90, 90, 120))
ERROR = (400, 300, 880, 500, (180, 40, 40))
CONTROL = (100, 300, 400, 600, (60, 160, 60))
# The session before it rests on the desktop, as run 42074769175 caught it: a
# black desktop under the top bar, the startup animation, and the overview.
UNPAINTED = [(0, 0, W - 1, H - 1, (0, 0, 0)), (0, 0, W - 1, 30, (40, 40, 40))]
OVERVIEW = (300, 150, 980, 650, (120, 90, 140))
FRAMES = {
    "desktop": [],
    "open": [WINDOW],
    "typed": [WINDOW, QUERY],
    "panel": [WINDOW, QUERY, PANEL],
    "error": [ERROR],
    "control": [CONTROL],
    "unpainted": UNPAINTED,
    "overview": [OVERVIEW],
    "anim-a": [(200, 200, 500, 500, (150, 150, 150))],
    "anim-b": [(600, 250, 900, 550, (150, 150, 150))],
}

# Small budgets, so a hang is seen to be stopped in seconds. Every scenario
# overrides what it is about; these are the rest.
FAST = {
    "VMTEST_SSH_CONNECT_S": "3",
    "VMTEST_GUEST_BUDGET_S": "30",
    "VMTEST_PROBE_BUDGET_S": "5",
    "VMTEST_CORRAL_BUDGET_S": "10",
    "VMTEST_DIAGNOSTICS_BUDGET_S": "10",
    "VMTEST_SETTLE_BUDGET_S": "20",
    "VMTEST_POLL_S": "0.5",
    "VMTEST_KILL_GRACE_S": "1",
}

# How much later than its budget a stopped call may be noticed. Generous,
# because scenarios share the machine with each other and with framediff.
SLACK_S = 6


def write_png(path: Path, rects) -> None:
    """Minimal RGB8 PNG writer, painting whole row spans rather than pixels."""
    blank = bytes(DESKTOP) * W
    rows = bytearray()
    for y in range(H):
        row = bytearray(blank)
        for x0, y0, x1, y1, colour in rects:
            if y0 <= y <= y1:
                row[x0 * 3 : (x1 + 1) * 3] = bytes(colour) * (x1 - x0 + 1)
        rows.append(0)
        rows += row

    def chunk(kind: bytes, body: bytes) -> bytes:
        return (
            struct.pack(">I", len(body))
            + kind
            + body
            + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF)
        )

    path.write_bytes(
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", W, H, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(bytes(rows)))
        + chunk(b"IEND", b"")
    )


def alive(pid: int) -> bool:
    """True for a process that exists and is not a zombie."""
    try:
        state = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[0]
    except (FileNotFoundError, ProcessLookupError, IndexError):
        return False
    return state != "Z"


class Run:
    """One launcher.sh run against one staged guest."""

    def __init__(self, frames: Path, name: str, scenario: dict, env: dict | None = None,
                 real_ssh_port: int | None = None) -> None:
        self.name = name
        self.dir = Path(tempfile.mkdtemp(prefix=f"launcher-selftest-{name}-"))
        self.out = self.dir / "out"
        self.out.mkdir()
        self.state = self.dir / "state"
        self.state.mkdir()
        bin_dir = self.dir / "bin"
        bin_dir.mkdir()
        (self.state / "scenario.json").write_text(json.dumps(scenario))

        if real_ssh_port is None:
            (bin_dir / "ssh").symlink_to(FAKEVM)
        (bin_dir / "corral").symlink_to(FAKEVM)
        # sudo is a pass-through: what is under test is the `timeout` launcher.sh
        # runs under it, not sudo.
        sudo = bin_dir / "sudo"
        sudo.write_text('#!/usr/bin/env bash\nif [ "${1:-}" = -E ]; then shift; fi\nexec "$@"\n')
        sudo.chmod(0o755)

        identity = self.dir / "id"
        identity.write_text("")
        identity.chmod(0o600)
        (self.out / "result.json").write_text(json.dumps({"ssh": {
            "identityFile": str(identity),
            "port": real_ssh_port or 2222,
            "user": "compass",
            "host": "127.0.0.1",
        }}))

        self.env = dict(os.environ)
        self.env.update(FAST)
        self.env.update(env or {})
        self.env.update({
            "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
            "FAKEVM_DIR": str(self.state),
            "FAKEVM_FRAMES": str(frames),
            "HOME": str(self.dir),
        })
        self.proc: subprocess.Popen | None = None
        self.output = ""
        self.status: int | None = None
        self.elapsed = 0.0
        self.ended = 0.0

    def start(self) -> None:
        self.started = time.time()
        # The guest's clock starts here: the pretend shell finishes starting
        # `shell_started_after` seconds after launcher.sh does.
        (self.state / "events.jsonl").write_text(
            json.dumps({"kind": "boot", "t": self.started}) + "\n")
        # A session of its own, so a cancellation test can signal launcher.sh
        # the way a runner would without signalling this file.
        self.proc = subprocess.Popen(
            ["bash", str(LAUNCHER), str(self.out), "testvm"],
            cwd=ROOT,
            env=self.env,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            start_new_session=True,
        )

    def finish(self, limit: float = 300) -> "Run":
        assert self.proc is not None
        try:
            self.output, _ = self.proc.communicate(timeout=limit)
        except subprocess.TimeoutExpired:
            os.killpg(self.proc.pid, signal.SIGKILL)
            self.output, _ = self.proc.communicate()
            self.output += f"\n[selftest] launcher.sh did not finish within {limit}s and was killed"
        self.ended = time.time()
        self.status = self.proc.returncode
        self.elapsed = self.ended - self.started
        return self

    def run(self) -> "Run":
        self.start()
        return self.finish()

    def calls(self) -> list[tuple[float, str]]:
        path = self.state / "calls.log"
        if not path.exists():
            return []
        out = []
        for line in path.read_text().splitlines():
            stamp, _, what = line.partition(" ")
            out.append((float(stamp), what))
        return out

    def first_call(self, prefix: str) -> float:
        for stamp, what in self.calls():
            if what.startswith(prefix):
                return stamp
        raise AssertionError(f"{self.name}: no call starting {prefix!r}\n{self.report()}")

    def hung(self) -> list[tuple[int, int, float, str]]:
        path = self.state / "hung-pids"
        if not path.exists():
            return []
        out = []
        for line in path.read_text().splitlines():
            pid, child, stamp, rule = line.split(" ", 3)
            out.append((int(pid), int(child), float(stamp), rule))
        return out

    def report(self) -> str:
        return (
            f"--- launcher.sh output for scenario {self.name} "
            f"(exit {self.status}, {self.elapsed:.1f}s) ---\n{self.output}"
        )


def real_ssh_scenario(frames: Path) -> Run:
    """A server that accepts the connection and never says a word.

    The case ConnectTimeout exists for: TCP succeeds, so nothing at the socket
    layer ever fails, and an ssh without a budget on the banner exchange sits
    there until something else kills it.
    """
    server = socket.socket()
    server.bind(("127.0.0.1", 0))
    server.listen(8)
    held = []

    def accept() -> None:
        while True:
            try:
                conn, _ = server.accept()
            except OSError:
                return
            held.append(conn)

    threading.Thread(target=accept, daemon=True).start()
    run = Run(frames, "silent-sshd", {}, {"VMTEST_SSH_CONNECT_S": "2",
                                          "VMTEST_GUEST_BUDGET_S": "120",
                                          "VMTEST_DIAGNOSTICS_BUDGET_S": "120"},
              real_ssh_port=server.getsockname()[1])
    try:
        return run.run()
    finally:
        server.close()
        for conn in held:
            conn.close()


def cancelled_scenario(frames: Path) -> Run:
    """SIGTERM to launcher.sh while a guest command hangs, as a runner would."""
    run = Run(frames, "cancelled", {"hang": ["ssh:engine-index"]},
              {"VMTEST_GUEST_BUDGET_S": "120"})
    run.start()
    deadline = time.time() + 120
    while not run.hung() and time.time() < deadline and run.proc.poll() is None:
        time.sleep(0.2)
    run.signalled = time.time()
    os.kill(run.proc.pid, signal.SIGTERM)
    return run.finish()


SCENARIOS = {
    # Paint lags its state by longer than the five seconds launcher.sh used to
    # sleep; the summon and the panel lag a little too. A fixed pause would
    # screenshot a hide that has not painted yet.
    "delayed-paint": lambda f: Run(f, "delayed-paint", {
        "paint_delay": {"hide": 7, "summon": 2, "panel": 2},
        "state_delay": {"hide": 1, "summon": 1, "panel": 1},
    }).run(),
    # Everything immediate: the waits must not cost a fixed five seconds.
    "prompt-paint": lambda f: Run(f, "prompt-paint", {}).run(),
    "frozen-error-screen": lambda f: Run(f, "frozen-error-screen",
                                         {"hide_paint": "error"},
                                         {"VMTEST_SETTLE_BUDGET_S": "6"}).run(),
    "launcher-never-hides": lambda f: Run(f, "launcher-never-hides",
                                          {"hide_state": "open", "hide_paint": "never"},
                                          {"VMTEST_SETTLE_BUDGET_S": "6"}).run(),
    "launcher-dies": lambda f: Run(f, "launcher-dies",
                                   {"hide_state": "open", "launcher_dies_on": "hide"},
                                   {"VMTEST_SETTLE_BUDGET_S": "120"}).run(),
    "guest-goes-away": lambda f: Run(f, "guest-goes-away", {"unreachable_after": "hide"},
                                     {"VMTEST_SETTLE_BUDGET_S": "120"}).run(),
    "hung-remote-command": lambda f: Run(f, "hung-remote-command",
                                         {"hang": ["ssh:engine-index"]},
                                         {"VMTEST_GUEST_BUDGET_S": "3"}).run(),
    "hung-probe": lambda f: Run(f, "hung-probe",
                                {"hang": ["ssh:ui-state:window=hidden"]},
                                {"VMTEST_PROBE_BUDGET_S": "2",
                                 "VMTEST_SETTLE_BUDGET_S": "7"}).run(),
    "hung-screenshot": lambda f: Run(f, "hung-screenshot",
                                     {"hang": ["corral:screenshot:launcher-04-hidden"]},
                                     {"VMTEST_CORRAL_BUDGET_S": "2",
                                      "VMTEST_SETTLE_BUDGET_S": "7"}).run(),
    "failed-gate-hung-collector": lambda f: Run(f, "failed-gate-hung-collector",
                                                {"fail": {"engine-index": 1},
                                                 "hang": ["ssh:journal"]},
                                                {"VMTEST_DIAGNOSTICS_BUDGET_S": "3"}).run(),
    "silent-sshd": real_ssh_scenario,
    "cancelled": cancelled_scenario,
    # #345: the shell finishes starting four seconds after the run begins, as
    # it did after corral's boot marker in run 42074769175, and its startup and
    # the overview's closing each animate for three seconds after that. The
    # unpainted desktop before it holds perfectly still, so a wait for stillness
    # alone would keep it.
    "slow-session": lambda f: Run(f, "slow-session",
                                  {"shell_started_after": 4, "settling_for": 3}).run(),
    "session-never-starts": lambda f: Run(f, "session-never-starts",
                                          {"shell_started_after": 1000},
                                          {"VMTEST_SESSION_BUDGET_S": "4"}).run(),
    "desktop-never-still": lambda f: Run(f, "desktop-never-still", {"settling_for": 1000},
                                         {"VMTEST_SETTLE_BUDGET_S": "5"}).run(),
}

RESULTS: dict[str, concurrent.futures.Future] = {}
SENTINEL: subprocess.Popen | None = None
_POOL: concurrent.futures.ThreadPoolExecutor | None = None
_FRAMES_DIR: Path | None = None


def setUpModule() -> None:
    global SENTINEL, _POOL, _FRAMES_DIR
    _FRAMES_DIR = Path(tempfile.mkdtemp(prefix="launcher-selftest-frames-"))
    for name, rects in FRAMES.items():
        write_png(_FRAMES_DIR / f"{name}.png", rects)
    # Unrelated to every run: nothing launcher.sh stops may include it.
    SENTINEL = subprocess.Popen(["sleep", "900"])
    workers = max(2, min(len(SCENARIOS), os.cpu_count() or 2))
    _POOL = concurrent.futures.ThreadPoolExecutor(max_workers=workers)
    for name, scenario in SCENARIOS.items():
        RESULTS[name] = _POOL.submit(scenario, _FRAMES_DIR)


def tearDownModule() -> None:
    if _POOL is not None:
        _POOL.shutdown(wait=True)
    if SENTINEL is not None:
        SENTINEL.kill()
        SENTINEL.wait()
    for future in RESULTS.values():
        try:
            shutil.rmtree(future.result().dir, ignore_errors=True)
        except Exception:  # noqa: BLE001 - a crashed scenario already failed its test
            pass
    if _FRAMES_DIR is not None:
        shutil.rmtree(_FRAMES_DIR, ignore_errors=True)


class Bounds(unittest.TestCase):
    maxDiff = None

    def result(self, name: str) -> Run:
        return RESULTS[name].result()

    def assertOutput(self, run: Run, needle: str) -> None:  # noqa: N802
        self.assertIn(needle, run.output, f"expected {needle!r}\n{run.report()}")

    def assertStatus(self, run: Run, status: int) -> None:  # noqa: N802
        self.assertEqual(run.status, status, run.report())

    def assertReaped(self, run: Run) -> None:  # noqa: N802
        """Every hung stub and its grandchild are gone; the sentinel is not."""
        hung = run.hung()
        self.assertTrue(hung, f"the scenario never hung, so it proves nothing\n{run.report()}")
        deadline = time.time() + 5
        while time.time() < deadline and any(alive(p) or alive(c) for p, c, _, _ in hung):
            time.sleep(0.1)
        for pid, child, _, rule in hung:
            self.assertFalse(alive(pid), f"{rule}: the hung call {pid} outlived the run\n{run.report()}")
            self.assertFalse(alive(child), f"{rule}: its child {child} outlived the run\n{run.report()}")
        self.assertTrue(alive(SENTINEL.pid), "an unrelated process was stopped by the run")

    def assertStoppedWithin(self, run: Run, budget: float, rule: str | None = None) -> None:  # noqa: N802
        """The (first) hung call ended no later than its budget, plus slack."""
        hung = [h for h in run.hung() if rule is None or h[3] == rule]
        self.assertTrue(hung, run.report())
        began = hung[0][2]
        self.assertLessEqual(run.ended - began, budget + SLACK_S,
                             f"stopped {run.ended - began:.1f}s after it hung, budget {budget}s\n"
                             + run.report())

    # -- positive -----------------------------------------------------------

    def test_paint_later_than_the_old_fixed_pause_still_passes(self):
        run = self.result("delayed-paint")
        self.assertStatus(run, 0)
        self.assertOutput(run, "ok: the launcher to report its window hidden")
        self.assertOutput(run, "ok: the desktop, repainted without the launcher")
        self.assertOutput(run, "ok: the launcher, painted again over the desktop")
        self.assertOutput(run, "ok: the action panel, painted over the list")
        # It waited through frames that were not ready yet, and kept them.
        attempts = sorted((run.out / "attempts").glob("launcher-04-hidden-*.png"))
        self.assertGreater(len(attempts), 1, run.report())

    def test_prompt_paint_costs_no_fixed_pause(self):
        run = self.result("prompt-paint")
        self.assertStatus(run, 0)
        # From each action to the first screenshot of what it should produce
        # there is now only a state probe. The fixed pause made that at least
        # five seconds every time, whatever the screen was doing.
        for action, frame in (("ssh window-attached", "launcher-04-hidden-01"),
                              ("ssh summon", "launcher-05-summoned-01"),
                              ("corral key", "launcher-06-panel-01")):
            acted = run.first_call(action)
            looked = next((t for t, what in run.calls()
                           if what.startswith("corral screenshot") and frame in what), None)
            self.assertIsNotNone(looked, f"no screenshot for {frame}\n{run.report()}")
            self.assertLess(looked - acted, 5, f"{action} -> {frame}\n{run.report()}")

    def test_ssh_carries_connection_and_keepalive_budgets(self):
        run = self.result("prompt-paint")
        argv = json.loads((run.state / "ssh-argv.jsonl").read_text().splitlines()[0])
        for option in ("ConnectTimeout=3", "ServerAliveInterval=5", "ServerAliveCountMax=3",
                       "BatchMode=yes"):
            self.assertIn(option, argv)

    def test_no_fixed_settle_remains(self):
        code = [line for line in LAUNCHER.read_text().splitlines()
                if not line.lstrip().startswith("#")]
        self.assertFalse([line for line in code if re.match(r"\s*sleep\s+[0-9]", line)],
                         "launcher.sh sleeps a fixed time again instead of waiting for a state")

    # -- wrong screen, wrong state, dead child, absent guest ----------------

    def test_frozen_error_screen_fails_on_paint_not_on_stability(self):
        run = self.result("frozen-error-screen")
        self.assertStatus(run, 124)
        self.assertOutput(run, "ok: the launcher to report its window hidden")
        self.assertOutput(run, "FAIL: TIMEOUT: the desktop, repainted without the launcher "
                               "was not on screen within 6s")
        # The frame it gave up on is kept under the gate's name, for the reader.
        self.assertTrue((run.out / "launcher-04-hidden.png").exists(), run.report())
        self.assertOutput(run, "collecting the guest's journal")

    def test_wrong_semantic_state_fails_on_state(self):
        run = self.result("launcher-never-hides")
        self.assertStatus(run, 124)
        self.assertOutput(run, "FAIL: TIMEOUT: the launcher to report its window hidden "
                               "did not happen within 6s")
        self.assertOutput(run, "FAIL: expected window=hidden")

    def test_dead_launcher_fails_at_once_and_says_so(self):
        run = self.result("launcher-dies")
        self.assertStatus(run, 1)
        self.assertOutput(run, "FAIL: the launcher exited while waiting for the launcher "
                               "to report its window hidden")
        # Not the 120 s settle budget: an exited launcher is not waited out.
        self.assertLess(run.ended - run.first_call("ssh window-attached"), 20, run.report())

    def test_absent_guest_fails_as_ssh_and_keeps_its_status(self):
        run = self.result("guest-goes-away")
        self.assertStatus(run, 255)
        self.assertOutput(run, "FAIL: the guest stopped answering SSH while waiting for")
        self.assertOutput(run, "SECONDARY: collecting the journal failed with status 255")
        self.assertLess(run.ended - run.first_call("ssh window-attached"), 20, run.report())

    # -- hung calls ---------------------------------------------------------

    def test_hung_remote_command_is_stopped_within_its_budget(self):
        run = self.result("hung-remote-command")
        self.assertStatus(run, 124)
        self.assertOutput(run, "TIMEOUT: the guest command "
                               "`/usr/libexec/compass-vmtest/checks.sh engine-index` "
                               "did not finish within its 3s budget")
        self.assertStoppedWithin(run, 3 + 1 + 10)  # + grace + the journal it then collects
        self.assertReaped(run)

    def test_hung_probe_is_stopped_and_the_wait_still_ends(self):
        run = self.result("hung-probe")
        self.assertStatus(run, 124)
        self.assertOutput(run, "did not finish within its 2s budget")
        self.assertOutput(run, "FAIL: TIMEOUT: the launcher to report its window hidden")
        self.assertGreater(len(run.hung()), 1, "the wait made one attempt, so it did not retry\n"
                           + run.report())
        self.assertLessEqual(run.ended - run.first_call("ssh window-attached"),
                             7 + 1 + 10 + SLACK_S, run.report())
        self.assertReaped(run)

    def test_hung_screenshot_is_stopped_and_the_wait_still_ends(self):
        run = self.result("hung-screenshot")
        self.assertStatus(run, 124)
        self.assertOutput(run, "TIMEOUT: `corral screenshot` did not finish within its 2s budget")
        self.assertOutput(run, "FAIL: TIMEOUT: the desktop, repainted without the launcher")
        self.assertLessEqual(run.ended - run.first_call("ssh window-attached"),
                             7 + 1 + 10 + SLACK_S, run.report())
        self.assertReaped(run)

    def test_failed_gate_keeps_its_status_when_the_collector_hangs(self):
        run = self.result("failed-gate-hung-collector")
        self.assertStatus(run, 1)
        self.assertOutput(run, "=== the run failed (status 1)")
        self.assertOutput(run, "SECONDARY: collecting the journal failed with status 124")
        self.assertOutput(run, "The run's own status is 1")
        self.assertStoppedWithin(run, 3 + 1, "ssh:journal")
        self.assertReaped(run)

    def test_silent_ssh_server_is_bounded_by_connect_timeout(self):
        if shutil.which("ssh") is None:
            self.skipTest("no ssh client on this machine")
        run = self.result("silent-sshd")
        self.assertStatus(run, 255)
        self.assertOutput(run, "SSH: the guest did not answer")
        self.assertOutput(run, "SECONDARY: collecting the journal failed with status 255")
        # Two connections (the first command, then the journal), each bounded
        # by the 2 s ConnectTimeout -- not by the 120 s command budgets.
        self.assertLess(run.elapsed, 2 * 2 + 2 * SLACK_S, run.report())

    def test_cancellation_reaps_the_call_in_flight(self):
        run = self.result("cancelled")
        self.assertStatus(run, 143)
        self.assertOutput(run, "CANCELLED: the run received SIGTERM")
        self.assertOutput(run, "no diagnostics collected")
        self.assertLess(run.ended - run.signalled, 1 + SLACK_S, run.report())
        self.assertReaped(run)

    # -- the baseline frames wait for the session (#345) --------------------

    def test_baselines_wait_for_the_shell_and_for_still_frames(self):
        run = self.result("slow-session")
        self.assertStatus(run, 0)
        self.assertOutput(run, "ok: the session's GNOME Shell to finish starting up")
        self.assertOutput(run, "ok: the login frame held still")
        self.assertOutput(run, "ok: the bare desktop held still")
        # The shell was asked until it said so, and the screen was not looked
        # at, nor the overview dismissed, before it had.
        polls = [t for t, what in run.calls() if what.startswith("ssh shell-started")]
        self.assertGreater(len(polls), 1, run.report())
        self.assertGreater(run.first_call("corral screenshot"), polls[-1], run.report())
        self.assertGreater(run.first_call("ssh overview-dismiss"), polls[-1], run.report())
        # What was kept is the session at rest: the overview it opened at login,
        # then the desktop once that had finished closing -- never the unpainted
        # desktop, though it held still, nor a frame of either animation.
        frames = Path(run.env["FAKEVM_FRAMES"])
        self.assertEqual((run.out / "launcher-00z-login.png").read_bytes(),
                         (frames / "overview.png").read_bytes(), run.report())
        self.assertEqual((run.out / "launcher-00a-bare-desktop.png").read_bytes(),
                         (frames / "desktop.png").read_bytes(), run.report())
        # It waited through the closing animation's frames rather than keeping
        # the first three it saw.
        attempts = list((run.out / "attempts").glob("launcher-00a-bare-desktop-*.png"))
        self.assertGreater(len(attempts), 3, run.report())

    def test_a_shell_that_never_starts_fails_before_any_frame(self):
        run = self.result("session-never-starts")
        self.assertStatus(run, 124)
        self.assertOutput(run, "FAIL: TIMEOUT: the session's GNOME Shell to finish starting "
                               "up did not happen within 4s")
        self.assertFalse([what for _, what in run.calls() if what.startswith("corral")],
                         run.report())
        self.assertOutput(run, "collecting the guest's journal")

    def test_a_desktop_that_never_holds_still_fails_on_stillness(self):
        run = self.result("desktop-never-still")
        self.assertStatus(run, 124)
        self.assertOutput(run, "FAIL: TIMEOUT: the login frame did not hold still within 5s")
        self.assertTrue((run.out / "launcher-00z-login.png").exists(), run.report())
        attempts = list((run.out / "attempts").glob("launcher-00z-login-*.png"))
        self.assertGreater(len(attempts), 3, run.report())


if __name__ == "__main__":
    unittest.main(verbosity=2)
