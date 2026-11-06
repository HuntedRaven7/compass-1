#!/usr/bin/env python3
"""Compare this run's performance figures with a baseline series from `main`.

Run:
  perf-gate.py stamp IN.jsonl... --runner LABEL --sha SHA --out OUT.jsonl
  perf-gate.py extract LOG --profile P --renderer R --out OUT.jsonl
  perf-gate.py measure-rss --pid PID --metric M --renderer R --profile P --out OUT.jsonl
  perf-gate.py fetch --repo OWNER/NAME --dest DIR [--limit N] [--exclude-run ID]
  perf-gate.py compare --current CUR.jsonl --baseline DIR_OR_FILE... [--blocking]
                       [--expect METRIC]... [--resets FILE]
  perf-gate.py analyse SERIES.jsonl... [--k K] [--window N]
  perf-gate.py --selftest

WHY THIS EXISTS

`cargo bench` exits zero whatever the number says, and so does a test that
prints a resident set. §8.8 wants Suite 4 "blocking on `main`" and #127 wants
an idle-RSS regression gate, and both need the same thing: a stored series,
measured on the machine that will judge it, and a tolerance taken from that
series rather than invented. docs/rust-engine/PERF-GATES.md has the analysis
the constants below come from; read it before changing them.

THE SHAPE OF A POINT

One JSON object per line:

  {"metric": "slas.ipc_roundtrip_ping", "statistic": "median", "unit": "ns",
   "value": 33512.4, "sha": "<head sha>", "ref": "refs/heads/main",
   "event": "push", "run_id": "...", "recorded_at": "2026-11-05T04:45:38Z",
   "env": {"runner": "ubuntu-24.04", "image": "ubuntu24", "cpu": "AMD EPYC 7763
           64-Core Processor", "cores": 4, "arch": "x86_64", "profile": "bench",
           "renderer": "none", "image_version": "20261101.360.1"}}

The measuring step writes `metric`, `statistic`, `unit`, `value` and the parts
of `env` only it knows (`profile`, `arch`, `renderer`); `stamp` adds the rest
on the machine that measured, because a later job runs on a different machine.

WHAT "COMPATIBLE" MEANS

Two points are compared only when every field in KEY_FIELDS matches. The CPU
model is in the key because the hosted `ubuntu-24.04` pool is not one machine:
122 `main` runs of the fuzzy row split into two clusters around 600 µs and
900 µs, a gap far larger than either cluster's spread. A baseline that mixes
them is either useless or wrong. A point from another environment is never
silently compared; the report says it was refused, and why.

WHAT IT DOES NOT DO

It does not make a pull request red for a regression -- PR runs report, and
only a push to `main` blocks (`--blocking`). It does refuse, on any event, a
run that is missing a figure it was told to expect: a gate that measured
nothing must not look like a gate that measured and was satisfied.
"""

from __future__ import annotations

import argparse
import contextlib
import datetime
import io
import json
import os
import pathlib
import platform
import re
import statistics
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor

# Fields of `env` that must match for two points to be compared. `image_version`
# is recorded and deliberately not keyed: the hosted image is rebuilt weekly,
# and keying on it would keep every series below MIN_POINTS forever.
KEY_FIELDS = ("runner", "image", "cpu", "cores", "arch", "profile", "renderer")

# What every point must carry before it can enter a series.
REQUIRED = ("metric", "statistic", "unit", "value", "sha", "recorded_at")

# The baseline is the most recent WINDOW compatible points, and the gate judges
# only once it has a full window. The replay in PERF-GATES.md is why: on the
# history with its one sustained step split out (`series-step-free.jsonl`),
# windows of 10, 15 and 20 points under-estimated the spread and flagged 8 of
# 194, 5 of 169 and 3 of 144 points at K = 4, each a red `main` with nothing
# behind it. Windows of 30 flagged none of 94.
WINDOW = 30
MIN_POINTS = WINDOW

# Tolerance = K * (1.4826 * MAD) above the median. 1.4826 * MAD estimates the
# standard deviation of normal noise while ignoring outliers. K = 4 is what the
# replay of 122 historical `main` runs chose. With a window of 30, K = 4 and
# K = 5 both flagged none of the 94 step-free points. On the unsplit history
# K = 4 also flagged the first point of the sustained step (+8.9%, the fuzzy
# row moving from 878 to 950 µs and staying there), which K = 5 let through.
# Both flag the one lone +21.4% point.
K = 4.0
MAD_TO_SIGMA = 1.4826

# The minimum tolerance, as a fraction of the median. It exists for one case: a
# window whose MAD is zero (a run of identical page-granular RSS readings), in
# which any increase at all would block `main`. It is not the operative
# tolerance on any series measured so far -- the smallest K-sigma in the replay
# is 6.5% -- and it must stay well below that.
MIN_RELATIVE = 0.01

STATUS_OK = "ok"
STATUS_REGRESSED = "regressed"
STATUS_NO_BASELINE = "no-baseline"
STATUS_INSUFFICIENT = "insufficient"
STATUS_INCOMPATIBLE = "incompatible"

# Lines a measuring script prints for `extract` to collect, e.g. from the VM
# tier's `launcher-rss` check: `perf-point window.idle_rss_kb 138240 kB`.
PERF_LINE = re.compile(r"^perf-point (\S+) (\d+(?:\.\d+)?) (\S+)\s*$")


class PointError(ValueError):
    """A point that cannot enter a series, and the reason."""


# --------------------------------------------------------------------------
# Reading and writing points


def load_points(paths: list[pathlib.Path]) -> list[dict]:
    """Every point in the given files, or in `*.jsonl` files under directories."""
    points = []
    for path in paths:
        files = sorted(path.rglob("*.jsonl")) if path.is_dir() else [path]
        for file in files:
            for number, line in enumerate(file.read_text().splitlines(), start=1):
                if not line.strip():
                    continue
                try:
                    point = json.loads(line)
                    validate(point)
                except (json.JSONDecodeError, PointError) as error:
                    raise PointError(f"{file}:{number}: {error}") from error
                points.append(point)
    return points


def validate(point: dict) -> None:
    """Raise PointError unless the point identifies what, where and which commit."""
    missing = [field for field in REQUIRED if field not in point]
    env = point.get("env")
    if not isinstance(env, dict):
        missing.append("env")
    else:
        missing += [f"env.{field}" for field in KEY_FIELDS if env.get(field) in (None, "")]
    if missing:
        raise PointError(f"point for {point.get('metric', '?')} lacks {', '.join(missing)}")
    if not isinstance(point["value"], (int, float)) or point["value"] <= 0:
        raise PointError(f"point for {point['metric']} has value {point['value']!r}")


def write_points(path: pathlib.Path, points: list[dict]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a") as handle:
        for point in points:
            handle.write(json.dumps(point, sort_keys=True) + "\n")


def key_of(point: dict) -> tuple:
    return tuple(point["env"][field] for field in KEY_FIELDS)


def describe_env(env: dict) -> str:
    return ", ".join(f"{field}={env[field]}" for field in KEY_FIELDS)


# --------------------------------------------------------------------------
# The environment, read on the machine that measured


def cpu_model() -> str:
    """The CPU model name the kernel reports, or the platform's best guess."""
    try:
        for line in pathlib.Path("/proc/cpuinfo").read_text().splitlines():
            name, _, value = line.partition(":")
            if name.strip() == "model name" and value.strip():
                return " ".join(value.split())
    except OSError:
        pass
    return platform.processor() or platform.machine()


def image_name() -> str:
    """The hosted image (`ubuntu24`), or the distribution and version locally."""
    if os.environ.get("ImageOS"):
        return os.environ["ImageOS"]
    try:
        fields = dict(
            line.split("=", 1)
            for line in pathlib.Path("/etc/os-release").read_text().splitlines()
            if "=" in line
        )
        return f"{fields.get('ID', '').strip(chr(34))}{fields.get('VERSION_ID', '').strip(chr(34))}"
    except OSError:
        return platform.system().lower()


def stamp(partial: dict, runner: str, sha: str) -> dict:
    """A measured figure, completed with where and from which commit it came."""
    point = dict(partial)
    env = dict(point.get("env", {}))
    env.setdefault("arch", platform.machine())
    env.update(
        runner=runner,
        image=image_name(),
        image_version=os.environ.get("ImageVersion", ""),
        cpu=cpu_model(),
        cores=os.cpu_count() or 0,
    )
    point["env"] = env
    point.update(
        sha=sha,
        ref=os.environ.get("GITHUB_REF", ""),
        event=os.environ.get("GITHUB_EVENT_NAME", "local"),
        run_id=os.environ.get("GITHUB_RUN_ID", ""),
        recorded_at=datetime.datetime.now(datetime.UTC).strftime("%Y-%m-%dT%H:%M:%SZ"),
    )
    validate(point)
    return point


def status_kb(pid: int, field: str) -> int | None:
    try:
        for line in pathlib.Path(f"/proc/{pid}/status").read_text().splitlines():
            if line.startswith(f"{field}:"):
                return int(line.split()[1])
    except (OSError, ValueError, IndexError):
        return None
    return None


# --------------------------------------------------------------------------
# The comparison


def scaled_mad(values: list[float]) -> float:
    centre = statistics.median(values)
    return MAD_TO_SIGMA * statistics.median([abs(value - centre) for value in values])


def tolerance(values: list[float], k: float = K) -> tuple[float, float]:
    """The baseline median and how far above it a figure may sit."""
    centre = statistics.median(values)
    return centre, max(k * scaled_mad(values), MIN_RELATIVE * centre)


def reset_after(resets: list[dict], metric: str) -> str | None:
    """The latest reset date for this metric; points before it are history."""
    dates = [reset["after"] for reset in resets if reset["metric"] == metric]
    return max(dates) if dates else None


def judge(current: dict, baseline: list[dict], resets: list[dict]) -> dict:
    """One verdict: whether `current` is within the compatible series' spread."""
    metric = current["metric"]
    same_metric = [
        point
        for point in baseline
        if point["metric"] == metric
        and point["statistic"] == current["statistic"]
        and point["unit"] == current["unit"]
        # A re-run of the same commit is not a baseline for itself.
        and point["sha"] != current["sha"]
    ]
    after = reset_after(resets, metric)
    if after is not None:
        same_metric = [point for point in same_metric if point["recorded_at"] >= after]

    verdict = {"metric": metric, "value": current["value"], "unit": current["unit"],
               "env": current["env"]}
    if not same_metric:
        return verdict | {"status": STATUS_NO_BASELINE}

    compatible = [point for point in same_metric if key_of(point) == key_of(current)]
    if not compatible:
        others = sorted({key_of(point) for point in same_metric}, key=str)
        return verdict | {"status": STATUS_INCOMPATIBLE, "others": others}

    compatible.sort(key=lambda point: point["recorded_at"])
    window = [point["value"] for point in compatible[-WINDOW:]]
    if len(window) < MIN_POINTS:
        return verdict | {"status": STATUS_INSUFFICIENT, "n": len(window)}

    centre, allowed = tolerance(window)
    status = STATUS_REGRESSED if current["value"] > centre + allowed else STATUS_OK
    return verdict | {"status": status, "n": len(window), "median": centre,
                      "tolerance": allowed}


def differing_fields(env: dict, other: tuple) -> str:
    """What separates this run's environment from a baseline's, field by field."""
    return ", ".join(
        f"{field} is {theirs} there and {env[field]} here"
        for field, theirs in zip(KEY_FIELDS, other)
        if theirs != env[field]
    )


def render(verdicts: list[dict], blocking: bool) -> list[str]:
    mode = "blocking (a run measuring main)" if blocking else "informational"
    lines = [
        (f"Performance gate, {mode}. Tolerance is the baseline median plus {K:g} x the "
         f"scaled MAD of the last {WINDOW} compatible `main` points, at least "
         f"{MIN_RELATIVE:.0%}; docs/rust-engine/PERF-GATES.md says why."),
        "",
        "| metric | this run | baseline median (n) | limit | verdict |",
        "|---|---|---|---|---|",
    ]
    notes = []
    for verdict in verdicts:
        value = f"{verdict['value']:,.1f} {verdict['unit']}"
        status = verdict["status"]
        if status in (STATUS_OK, STATUS_REGRESSED):
            change = (verdict["value"] - verdict["median"]) / verdict["median"]
            limit = verdict["median"] + verdict["tolerance"]
            mark = "✅ within tolerance" if status == STATUS_OK else "❌ regressed"
            lines.append(
                f"| {verdict['metric']} | {value} | {verdict['median']:,.1f} ({verdict['n']}) "
                f"| {limit:,.1f} (+{verdict['tolerance'] / verdict['median']:.1%}) "
                f"| {mark}, {change:+.1%} |"
            )
        elif status == STATUS_INSUFFICIENT:
            lines.append(
                f"| {verdict['metric']} | {value} | {verdict['n']} of {MIN_POINTS} points | — "
                "| ⚠️ not judged: too few compatible points to measure the spread |"
            )
        elif status == STATUS_NO_BASELINE:
            lines.append(
                f"| {verdict['metric']} | {value} | none | — "
                "| ⚠️ not judged: no `main` run has recorded this metric yet |"
            )
        else:
            lines.append(
                f"| {verdict['metric']} | {value} | other environments only | — "
                "| ⛔ refused: the only baseline is from another environment |"
            )
            for other in verdict["others"]:
                notes.append(
                    f"  * {verdict['metric']}: a baseline exists where "
                    f"{differing_fields(verdict['env'], other)}."
                )
    environments = sorted({describe_env(verdict["env"]) for verdict in verdicts})
    lines.append("")
    lines.append("Measured on: " + "; ".join(environments) + ".")
    if notes:
        lines.append("")
        lines.append("Not compared, because a figure from another environment is not a baseline:")
        lines += notes
    return lines


def compare(current: list[dict], baseline: list[dict], resets: list[dict],
            expect: list[str]) -> tuple[list[dict], list[str]]:
    """The verdicts, and the expected metrics this run did not produce."""
    produced = {point["metric"] for point in current}
    missing = [metric for metric in expect if metric not in produced]
    return [judge(point, baseline, resets) for point in current], missing


# --------------------------------------------------------------------------
# Fetching the series from previous runs on `main`

# The events whose runs on `main` measure `main` itself. A pull request from a
# fork's branch that happens to be called `main` reports that branch name too,
# so the event is checked as well as the branch.
SERIES_EVENTS = ("push", "schedule")


def fetch(repo: str, workflow: str, dest: pathlib.Path, limit: int, exclude: str) -> int:
    """Download `perf-points-*` artifacts from recent runs on `main`."""
    listed = subprocess.run(
        ["gh", "run", "list", "--repo", repo, "--workflow", workflow, "--branch", "main",
         "--status", "completed", "--limit", str(limit),
         "--json", "databaseId,conclusion,event"],
        check=False, capture_output=True, text=True,
    )
    if listed.returncode != 0:
        # The pull request that adds the workflow runs it before `main` has it,
        # and that is the only failure that means "no series yet". Anything else
        # (a token without `actions: read`, an API outage) must not quietly turn
        # every verdict into "no baseline".
        if "not found on the default branch" in listed.stderr:
            print(f"{workflow} is not on main yet, so there is no series to fetch.")
            return 0
        print(f"error: gh run list failed: {listed.stderr.strip()}", file=sys.stderr)
        return 1
    # A cancelled run was superseded before it measured; a failed one may still
    # have measured, and a regression it recorded is part of the series.
    runs = [
        str(run["databaseId"])
        for run in json.loads(listed.stdout)
        if run["event"] in SERIES_EVENTS
        and run["conclusion"] not in ("cancelled", "skipped")
        and str(run["databaseId"]) != exclude
    ]

    def download(run: str) -> bool:
        # Runs from before the gate existed have no such artifact, so a failed
        # download is expected and only counted.
        return subprocess.run(
            ["gh", "run", "download", run, "--repo", repo, "--pattern", "perf-points-*",
             "--dir", str(dest / run)],
            check=False, capture_output=True, text=True,
        ).returncode == 0

    with ThreadPoolExecutor(max_workers=8) as pool:
        fetched = sum(pool.map(download, runs))
    print(f"Fetched perf points from {fetched} of {len(runs)} recent runs on main.")
    return 0


# --------------------------------------------------------------------------
# The replay behind the constants


def analyse(points: list[dict], k: float = K, window: int = WINDOW) -> list[str]:
    """Replay each series as the gate would have seen it, one point at a time."""
    lines = [
        f"Replay with K = {k:g}, judging each point once {window} earlier points exist.",
        "",
        ("| metric | environment | points | median | scaled MAD | tolerance, smallest "
         "/ typical | flagged |"),
        "|---|---|---|---|---|---|---|",
    ]
    series: dict[tuple, list[dict]] = {}
    for point in points:
        series.setdefault((point["metric"], key_of(point)), []).append(point)
    for (metric, key), members in sorted(series.items()):
        members.sort(key=lambda point: point["recorded_at"])
        values = [point["value"] for point in members]
        flagged, relative = [], []
        for index in range(window, len(values)):
            centre, allowed = tolerance(values[max(0, index - window):index], k)
            relative.append(allowed / centre)
            if values[index] > centre + allowed:
                flagged.append(f"{(values[index] - centre) / centre:+.1%}")
        centre = statistics.median(values)
        spread = (
            f"{min(relative):.1%} / {statistics.median(relative):.1%}" if relative else "—"
        )
        lines.append(
            f"| {metric} | {describe_env(dict(zip(KEY_FIELDS, key)))} | {len(values)} "
            f"| {centre:,.1f} | {scaled_mad(values) / centre:.2%} | {spread} "
            f"| {len(flagged)} of {len(relative)}"
            f"{': ' + ', '.join(flagged) if flagged else ''} |"
        )
    return lines


# --------------------------------------------------------------------------
# Command line


def main(argv: list[str]) -> int:
    if argv[1:] == ["--selftest"]:
        return selftest()

    parser = argparse.ArgumentParser(prog="perf-gate.py")
    commands = parser.add_subparsers(dest="command", required=True)

    stamp_cmd = commands.add_parser("stamp", help="complete measured figures with their environment")
    stamp_cmd.add_argument("inputs", nargs="+", type=pathlib.Path)
    stamp_cmd.add_argument("--runner", required=True, help="the runs-on label that measured")
    stamp_cmd.add_argument("--sha", required=True, help="the head commit that was measured")
    stamp_cmd.add_argument("--out", required=True, type=pathlib.Path)

    extract_cmd = commands.add_parser("extract", help="collect `perf-point` lines from a log")
    extract_cmd.add_argument("log", type=pathlib.Path)
    extract_cmd.add_argument("--profile", required=True)
    extract_cmd.add_argument("--renderer", required=True)
    extract_cmd.add_argument("--out", required=True, type=pathlib.Path)

    measure_cmd = commands.add_parser("measure-rss", help="record a running process's idle RSS")
    measure_cmd.add_argument("--pid", required=True, type=int)
    measure_cmd.add_argument("--metric", required=True)
    measure_cmd.add_argument("--renderer", required=True)
    measure_cmd.add_argument("--profile", required=True)
    measure_cmd.add_argument("--runner", required=True)
    measure_cmd.add_argument("--sha", required=True)
    measure_cmd.add_argument("--out", required=True, type=pathlib.Path)

    fetch_cmd = commands.add_parser("fetch", help="download the series from recent main runs")
    fetch_cmd.add_argument("--repo", required=True)
    fetch_cmd.add_argument("--workflow", default="perf.yaml")
    fetch_cmd.add_argument("--dest", required=True, type=pathlib.Path)
    # Enough runs that the rarer of the pool's CPU models can fill a WINDOW: the
    # replay in PERF-GATES.md saw it on 36 of 122 runs, so 150 runs hold ~44.
    fetch_cmd.add_argument("--limit", type=int, default=150)
    fetch_cmd.add_argument("--exclude-run", default="")

    compare_cmd = commands.add_parser("compare", help="judge this run against the series")
    compare_cmd.add_argument("--current", required=True, nargs="+", type=pathlib.Path)
    compare_cmd.add_argument("--baseline", required=True, nargs="+", type=pathlib.Path)
    compare_cmd.add_argument("--expect", action="append", default=[])
    compare_cmd.add_argument("--resets", type=pathlib.Path)
    compare_cmd.add_argument("--blocking", action="store_true")

    analyse_cmd = commands.add_parser("analyse", help="replay a series to calibrate the gate")
    analyse_cmd.add_argument("series", nargs="+", type=pathlib.Path)
    analyse_cmd.add_argument("--k", type=float, default=K)
    analyse_cmd.add_argument("--window", type=int, default=WINDOW)

    args = parser.parse_args(argv[1:])
    try:
        return run(args)
    except PointError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1


def run(args: argparse.Namespace) -> int:
    if args.command == "stamp":
        partial = []
        for path in args.inputs:
            partial += [json.loads(line) for line in path.read_text().splitlines() if line.strip()]
        if not partial:
            raise PointError(f"{', '.join(map(str, args.inputs))} held no figures to stamp")
        write_points(args.out, [stamp(point, args.runner, args.sha) for point in partial])
        print(f"Stamped {len(partial)} points into {args.out}.")
        return 0

    if args.command == "extract":
        points = []
        for line in args.log.read_text(errors="replace").splitlines():
            match = PERF_LINE.match(line.strip())
            if match:
                points.append({
                    "metric": match.group(1), "statistic": "single", "unit": match.group(3),
                    "value": float(match.group(2)),
                    "env": {"profile": args.profile, "renderer": args.renderer},
                })
        if not points:
            raise PointError(f"{args.log} has no `perf-point` lines")
        write_points(args.out, points)
        print(f"Extracted {len(points)} points into {args.out}.")
        return 0

    if args.command == "measure-rss":
        rss = status_kb(args.pid, "VmRSS")
        if rss is None:
            raise PointError(f"process {args.pid} has no readable VmRSS")
        partial = {"metric": args.metric, "statistic": "single", "unit": "kB",
                   "value": float(rss),
                   "env": {"profile": args.profile, "renderer": args.renderer}}
        write_points(args.out, [stamp(partial, args.runner, args.sha)])
        print(f"{args.metric}: VmRSS {rss} kB, recorded in {args.out}.")
        return 0

    if args.command == "fetch":
        return fetch(args.repo, args.workflow, args.dest, args.limit, args.exclude_run)

    if args.command == "analyse":
        print("\n".join(analyse(load_points(args.series), args.k, args.window)))
        return 0

    current = load_points(args.current)
    baseline = load_points([path for path in args.baseline if path.exists()])
    resets = json.loads(args.resets.read_text())["resets"] if args.resets else []
    for reset in resets:
        # A reset with no reason is a regression waved through with no record
        # of why, which is the thing this file exists to prevent.
        if not all(reset.get(field) for field in ("metric", "after", "reason")):
            raise PointError(f"{args.resets}: reset {reset!r} needs a metric, an after and a reason")
    verdicts, missing = compare(current, baseline, resets, args.expect)
    print("\n".join(render(verdicts, args.blocking)))
    if missing:
        print(
            f"error: this run produced no figure for {', '.join(missing)}. The measuring step "
            "did not run or renamed its metric, and a gate that measured nothing must not "
            "pass as one that measured and was satisfied.",
            file=sys.stderr,
        )
        return 1
    regressed = [verdict["metric"] for verdict in verdicts if verdict["status"] == STATUS_REGRESSED]
    for metric in regressed:
        level = "error" if args.blocking else "warning"
        print(f"::{level}::{metric} regressed beyond the spread measured on main.")
    return 1 if regressed and args.blocking else 0


# --------------------------------------------------------------------------
# Self-test


def selftest() -> int:
    env = {"runner": "ubuntu-24.04", "image": "ubuntu24", "cpu": "AMD EPYC 7763", "cores": 4,
           "arch": "x86_64", "profile": "bench", "renderer": "none"}

    def point(value: float, day: int, sha: str = "", **changes) -> dict:
        return {"metric": "slas.row", "statistic": "median", "unit": "ns", "value": value,
                "sha": sha or f"sha{day}", "recorded_at": f"2026-11-{day:02d}T00:00:00Z",
                "env": env | changes}

    # A series with real spread: median 1000, MAD 10, so the limit is 1000 + 5 * 14.8.
    series = [point(value, day) for day, value in
              enumerate([980, 990, 1010, 1000, 990, 1000, 1020, 1000, 1010, 1000] * 3, start=1)]
    limit = 1000 + K * MAD_TO_SIGMA * 10

    def status(current: dict, baseline: list[dict], resets: list[dict] | None = None) -> str:
        return judge(current, baseline, resets or [])["status"]

    checks = {
        "a compatible figure inside the spread passes":
            status(point(limit - 1, 20, "new"), series) == STATUS_OK,
        "a compatible figure beyond the spread is a regression":
            status(point(limit + 1, 20, "new"), series) == STATUS_REGRESSED,
        "a faster figure is never a regression":
            status(point(10, 20, "new"), series) == STATUS_OK,
        "no point for the metric at all is reported, not judged":
            status(point(9e9, 20, "new"), []) == STATUS_NO_BASELINE,
        "a series on another CPU is refused, however different the number":
            status(point(9e9, 20, "new", cpu="Intel Xeon 8370C"), series) == STATUS_INCOMPATIBLE,
        "a series in another build profile is refused":
            status(point(9e9, 20, "new", profile="debug"), series) == STATUS_INCOMPATIBLE,
        "a series under another renderer is refused":
            status(point(9e9, 20, "new", renderer="llvmpipe"), series) == STATUS_INCOMPATIBLE,
        "too few compatible points are reported, not judged":
            status(point(9e9, 20, "new"), series[: MIN_POINTS - 1]) == STATUS_INSUFFICIENT,
        "a re-run of a baseline commit is not judged against itself":
            status(point(9e9, 20, "sha1"), series[:MIN_POINTS]) == STATUS_INSUFFICIENT,
        "a reset drops the history before it":
            status(point(9e9, 20, "new"), series,
                   [{"metric": "slas.row", "after": "2026-11-05T00:00:00Z"}])
            == STATUS_INSUFFICIENT,
        "a zero-spread series still allows the minimum tolerance":
            status(point(1000 * (1 + MIN_RELATIVE) - 1, 20, "new"),
                   [point(1000, day) for day in range(1, WINDOW + 1)]) == STATUS_OK,
        "only the latest WINDOW points form the baseline":
            status(point(1100, 90, "new"),
                   [point(5000, day) for day in range(1, 40)]
                   + [point(1000 + day % 3, 40 + day) for day in range(WINDOW)])
            == STATUS_REGRESSED,
    }

    try:
        validate({"metric": "m", "statistic": "median", "unit": "ns", "value": 1.0,
                  "sha": "s", "recorded_at": "t", "env": env | {"cpu": ""}})
    except PointError:
        checks["a point that does not say where it was measured is rejected"] = True
    else:
        checks["a point that does not say where it was measured is rejected"] = False

    # The command line end to end: exit codes are what CI acts on.
    with tempfile.TemporaryDirectory() as scratch:
        root = pathlib.Path(scratch)
        write_points(root / "base" / "run" / "points.jsonl", series)
        write_points(root / "ok.jsonl", [point(1000, 20, "new")])
        write_points(root / "bad.jsonl", [point(limit + 1, 20, "new")])

        def cli(*extra: str) -> int:
            # The report is not what is under test here, only the exit code.
            with contextlib.redirect_stdout(io.StringIO()), \
                    contextlib.redirect_stderr(io.StringIO()):
                return main(["perf-gate.py", "compare", *extra])

        base = ("--baseline", str(root / "base"))
        checks["a regression fails a blocking run"] = (
            cli(*base, "--current", str(root / "bad.jsonl"), "--blocking") == 1)
        checks["a regression does not fail an informational run"] = (
            cli(*base, "--current", str(root / "bad.jsonl")) == 0)
        checks["a passing blocking run exits zero"] = (
            cli(*base, "--current", str(root / "ok.jsonl"), "--blocking") == 0)
        checks["a run missing an expected metric fails, informational or not"] = (
            cli(*base, "--current", str(root / "ok.jsonl"), "--expect", "slas.other") == 1)
        checks["a missing baseline directory is no baseline, not a crash"] = (
            cli("--baseline", str(root / "absent"), "--current", str(root / "bad.jsonl"),
                "--blocking") == 0)

        log = root / "vm.log"
        log.write_text("noise\nperf-point window.idle_rss_kb 138240 kB\n"
                       "perf-point engine.idle_rss_kb 10064 kB\n")
        with contextlib.redirect_stdout(io.StringIO()):
            main(["perf-gate.py", "extract", str(log), "--profile", "flatpak",
                  "--renderer", "llvmpipe", "--out", str(root / "vm.jsonl")])
        extracted = [json.loads(line) for line in (root / "vm.jsonl").read_text().splitlines()]
        checks["engine and window figures are extracted under separate names"] = (
            [p["metric"] for p in extracted] == ["window.idle_rss_kb", "engine.idle_rss_kb"])

    stamped = stamp({"metric": "m", "statistic": "median", "unit": "ns", "value": 1.0,
                     "env": {"profile": "bench", "renderer": "none"}}, "local", "abc")
    checks["stamping fills every key field"] = all(stamped["env"].get(f) for f in KEY_FIELDS)

    failed = [name for name, passed in checks.items() if not passed]
    for name in failed:
        print(f"selftest: FAILED: {name}", file=sys.stderr)
    if failed:
        return 1
    print(f"selftest: ok ({len(checks)} checks)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
