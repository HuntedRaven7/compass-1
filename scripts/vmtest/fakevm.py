#!/usr/bin/env python3
"""A pretend guest and a pretend corral, for launcher-selftest.py only.

launcher.sh talks to the world through two programs: `ssh` (to run checks.sh
in the guest) and `corral` (to screenshot, type and press keys from the host).
This file is both. launcher-selftest.py links it into a scratch directory as
`ssh` and as `corral`, puts that directory first on PATH, and the name it was
run under says which part to play.

The two halves share one model of the guest, kept as an append-only event log
in $FAKEVM_DIR: launcher-start, the typed query, the hide toggle, the summon,
the Ctrl+B chord. The launcher's state line (what `checks.sh ui-state` reads)
and the screen (what `corral screenshot` captures) are both computed from that
log, each with its own delay, which is how a test makes paint lag behind state,
or state never arrive, or the screen freeze on the wrong picture.

$FAKEVM_DIR/scenario.json says how this guest misbehaves:

    paint_delay   {"hide": s, "summon": s, "panel": s}  seconds before it shows
    state_delay   the same, for the launcher's state line
    hide_paint    "desktop" (correct), "error" (a frozen dialog), "never"
    hide_state    "hidden" (correct), or "open" (the launcher never says so)
    launcher_dies_on   an event after which `launcher-status` reports an exit
    unreachable_after  an event after which every ssh fails as ssh does, 255
    hang          ["ssh:<subcommand>[:<needle>]", "corral:<verb>[:<needle>]"]
    fail          {"<subcommand>": exit status}
    shell_started_after  seconds from the run's start before GNOME Shell says
                  it has started (`checks.sh shell-started`); until then the
                  screen is an unpainted desktop that holds perfectly still
    settling_for  seconds after the shell starts, and again after the overview
                  is dismissed, during which no two screenshots are alike

Nothing here is a model of checks.sh beyond what launcher.sh reads back: the
state line's keys, launcher-status's exit, the shell's startup line, and frames
that pass or fail the real framediff gates for the reason the scenario names.
"""

from __future__ import annotations

import json
import os
import shlex
import shutil
import subprocess
import sys
import time
from pathlib import Path

STATE = Path(os.environ["FAKEVM_DIR"])
FRAMES = Path(os.environ["FAKEVM_FRAMES"])
CHECKS = "/usr/libexec/compass-vmtest/checks.sh"

# Which event each checks.sh subcommand or corral verb stands for.
SSH_EVENTS = {
    "launcher-start": "start",
    "window-attached": "hide",
    "summon": "summon",
    "control-app-start": "control",
    "overview-dismiss": "dismiss",
}


def scenario() -> dict:
    path = STATE / "scenario.json"
    return json.loads(path.read_text()) if path.exists() else {}


def log_call(line: str) -> None:
    with open(STATE / "calls.log", "a") as f:
        f.write(f"{time.time():.3f} {line}\n")


def record(kind: str) -> None:
    with open(STATE / "events.jsonl", "a") as f:
        f.write(json.dumps({"kind": kind, "t": time.time()}) + "\n")


def events() -> list[dict]:
    path = STATE / "events.jsonl"
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text().splitlines() if line]


def happened(kind: str) -> bool:
    return any(e["kind"] == kind for e in events())


def hang_if_asked(key: str, words: list[str]) -> None:
    """Hang for ten minutes if the scenario says this call hangs.

    A grandchild is started first and both pids are recorded, so the test can
    check that whatever stopped this call stopped what it started as well.
    """
    for rule in scenario().get("hang", []):
        parts = rule.split(":", 2)
        if ":".join(parts[:2]) != key:
            continue
        if len(parts) == 3 and parts[2] not in " ".join(words):
            continue
        child = subprocess.Popen(["sleep", "600"])
        with open(STATE / "hung-pids", "a") as f:
            f.write(f"{os.getpid()} {child.pid} {time.time():.3f} {rule}\n")
        time.sleep(600)
        sys.exit(99)


def effective(delays: str, now: float) -> list[str]:
    """The event kinds that have taken effect by `now`, in order."""
    table = scenario().get(delays, {})
    return [e["kind"] for e in events() if e["t"] + float(table.get(e["kind"], 0)) <= now]


def state_line() -> str:
    s = scenario()
    window, panel, query = "hidden", "closed", ""
    for kind in effective("state_delay", time.time()):
        if kind == "start":
            window = "open"
        elif kind == "type":
            query = os.environ.get("LAUNCHER_QUERY", "fi")
        elif kind == "hide":
            window = s.get("hide_state", "hidden")
        elif kind == "summon":
            window = "open"
        elif kind == "panel":
            panel = "open"
    return (
        "2026-11-05T00:00:00Z DEBUG compass_ui::state: "
        f'query="{query}" results=3 selected=0 panel={panel} panel_title="Open" window={window}'
    )


def shell_started_at() -> float:
    """When the pretend GNOME Shell finishes starting, from the run's start."""
    boot = next((e["t"] for e in events() if e["kind"] == "boot"), 0.0)
    return boot + float(scenario().get("shell_started_after", 0))


def session_frame(now: float) -> str | None:
    """What the shell has on screen of its own, or None once it rests on the desktop.

    The sequence run 42074769175 caught part-way: an unpainted desktop that
    holds still, the startup animation, the overview it opens at login, and the
    animation of its closing once it is dismissed.
    """
    started = shell_started_at()
    if now < started:
        return "unpainted"
    settling = float(scenario().get("settling_for", 0))
    dismissed = [e["t"] for e in events() if e["kind"] == "dismiss"]
    moving_since = dismissed[0] if dismissed else started
    if now < moving_since + settling:
        # Each screenshot differs from the one before it.
        counter = STATE / "animation-frames"
        count = int(counter.read_text()) if counter.exists() else 0
        counter.write_text(str(count + 1))
        return "anim-a" if count % 2 == 0 else "anim-b"
    return None if dismissed else "overview"


def screen() -> Path:
    shell = session_frame(time.time())
    if shell is not None:
        return FRAMES / f"{shell}.png"
    s = scenario()
    started = typed = panel = control = False
    window = "open"
    for kind in effective("paint_delay", time.time()):
        if kind == "start":
            started, window = True, "open"
        elif kind == "type":
            typed = True
        elif kind == "hide":
            window = {"desktop": "hidden", "error": "error", "never": window}[
                s.get("hide_paint", "desktop")
            ]
        elif kind == "summon":
            window = "open"
        elif kind == "panel":
            panel = True
        elif kind == "control":
            control = True
    if control:
        name = "control"
    elif not started or window == "hidden":
        name = "desktop"
    elif window == "error":
        name = "error"
    elif panel:
        name = "panel"
    elif typed:
        name = "typed"
    else:
        name = "open"
    return FRAMES / f"{name}.png"


def ssh(argv: list[str]) -> int:
    with open(STATE / "ssh-argv.jsonl", "a") as f:
        f.write(json.dumps(argv) + "\n")
    # Options that take a value, then the destination, then the remote command.
    i = 0
    while i < len(argv) and argv[i].startswith("-"):
        i += 2 if argv[i] in ("-i", "-p", "-o") else 1
    # ssh hands the remote side ONE string, which the remote shell splits
    # again. Emulated, because launcher.sh's quoting depends on it.
    words = shlex.split(" ".join(argv[i + 1 :]))
    s = scenario()
    sub = words[1] if words[:1] == [CHECKS] and len(words) > 1 else words[0]
    args = words[2:] if words[:1] == [CHECKS] else words[1:]
    log_call(f"ssh {sub} {' '.join(args)}")

    after = s.get("unreachable_after")
    if after and happened(after):
        print("ssh: connect to host 127.0.0.1 port 2222: Connection refused", file=sys.stderr)
        return 255
    hang_if_asked(f"ssh:{sub}", args)
    if sub in s.get("fail", {}):
        print(f"stub: {sub} fails as the scenario asks", file=sys.stderr)
        return int(s["fail"][sub])
    if sub == "shell-started":
        if time.time() < shell_started_at():
            print("GNOME Shell has not logged 'GNOME Shell started at' this boot yet")
            return 1
        print("bluefin gnome-shell[2779]: GNOME Shell started at (stub)")
        return 0
    if sub == "overview-dismiss" and time.time() < shell_started_at():
        # As checks.sh, which refuses: before the shell has started the overview
        # is closed only because the startup animation has not opened it yet.
        print("refusing to dismiss the overview: GNOME Shell has not started", file=sys.stderr)
        return 1
    if sub in SSH_EVENTS:
        record(SSH_EVENTS[sub])

    if sub == "ui-state":
        line = state_line()
        print("--- every state the launcher passed through ---")
        print(f"     1\t{line}")
        print("--- asserting against the last line ---")
        print(f"  {line}")
        status = 0
        for want in args:
            if want in line:
                print(f"  ok: {want}")
            else:
                print(f"  FAIL: expected {want}", file=sys.stderr)
                status = 1
        return status
    if sub == "launcher-status":
        dies_on = s.get("launcher_dies_on")
        if dies_on and happened(dies_on):
            print("the launcher exited 101", file=sys.stderr)
            return 1
        print("the launcher is still running")
        return 0
    if sub in ("test", "cat"):
        return 1
    print(f"stub: {sub} ok")
    return 0


def corral(argv: list[str]) -> int:
    verb = argv[0]
    log_call(f"corral {' '.join(argv)}")
    hang_if_asked(f"corral:{verb}", argv[1:])
    if verb == "screenshot":
        target = argv[argv.index("-o") + 1]
        shutil.copyfile(screen(), target)
        return 0
    if verb == "type":
        record("type")
        return 0
    if verb == "key":
        record("panel")
        return 0
    print(f"stub corral: unknown verb {verb}", file=sys.stderr)
    return 2


def main() -> int:
    role = Path(sys.argv[0]).name
    if role == "ssh":
        return ssh(sys.argv[1:])
    if role == "corral":
        return corral(sys.argv[1:])
    print(f"fakevm.py must be run as ssh or corral, not {role}", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())
