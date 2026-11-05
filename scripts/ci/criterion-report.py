#!/usr/bin/env python3
"""Read a criterion run's estimates and report them against the SLA table.

Run: python3 scripts/ci/criterion-report.py [target/criterion/slas] [--points PATH]
     python3 scripts/ci/criterion-report.py --selftest

WHY THIS EXISTS

`cargo bench` prints a number and exits zero whatever the number says. That is
how a 24x miss sat unnoticed in `compass-ipc`'s bench, and it is why §8.5 asks
for "SLAs enforced as CI failures, not advisory numbers". A CI step that only
ran the bench would be green on a run that measured nothing at all -- the
defect shape this repository has now found five times.

So this asserts the rows it expects were actually measured, prints each median
against the SLA the table names, and says which rows the harness cannot reach.

WHAT IT DOES NOT DO

It does not gate. The thresholds below are the SLAs, and a *threshold* is not
the same thing as a *regression gate*. With `--points PATH` it writes each
row's median as a point for `perf-gate.py`, which judges it against the series
from earlier `main` runs; docs/rust-engine/PERF-GATES.md explains that gate.
The median, not the mean criterion prints: the two differ by ~6% on the IPC
row, and the series is only comparable if every point uses the same statistic.
"""

import json
import pathlib
import platform
import sys

# The rows `benches/slas.rs` measures, and the SLA each one is named against in
# PLAN.md §8.5. Nanoseconds, because that is criterion's unit.
ROWS = {
    "fuzzy_rank_top20_of_10k": ("Fuzzy search, top-20 of 10,000 items", 2_000_000.0),
    "ipc_roundtrip_ping": ("IPC round-trip, local UDS", 500_000.0),
}

# What §8.5 names that this harness cannot measure, and why. Printed rather
# than silently absent: a report listing two rows out of six looks like a
# passing run unless it says what the other four are.
UNREACHABLE = {
    "Cold start to first frame": "needs a display; the VM tier reports a floor",
    "Summon to first frame": "needs a compositor and a portal activation",
    "Idle RSS": "a property of the shipped process, not of a bench binary",
    "Peak RSS, 10k index + 3 extensions": "the same, plus a worker host",
}


def medians(root: pathlib.Path) -> dict[str, float]:
    """Each benchmark's median, in nanoseconds, keyed by its directory name."""
    found = {}
    for estimates in sorted(root.glob("*/new/estimates.json")):
        name = estimates.parent.parent.name
        with estimates.open() as handle:
            found[name] = json.load(handle)["median"]["point_estimate"]
    return found


def report(found: dict[str, float]) -> list[str]:
    """The lines to print, and an empty list is not a success."""
    lines = ["| row | median | SLA | |", "|---|---|---|---|"]
    for key, (label, sla) in ROWS.items():
        value = found[key]
        verdict = "✅" if value < sla else "❌"
        lines.append(
            f"| {label} | {value / 1000:.1f} µs | < {sla / 1000:.0f} µs | {verdict} |"
        )
    lines.append("")
    lines.append("Not measurable in this harness, and not faked into it:")
    for label, why in UNREACHABLE.items():
        lines.append(f"  * {label} — {why}")
    return lines


def main(argv: list[str]) -> int:
    if "--selftest" in argv:
        return selftest()

    args = argv[1:]
    points_path = None
    if "--points" in args:
        at = args.index("--points")
        points_path = pathlib.Path(args[at + 1])
        del args[at : at + 2]
    root = pathlib.Path(args[0] if args else "target/criterion/slas")
    found = medians(root)

    missing = [key for key in ROWS if key not in found]
    if missing:
        print(
            f"error: {root} has no estimates for {missing}. The bench did not "
            "run, or its row names changed -- either way this report would "
            "otherwise be a green step that measured nothing.",
            file=sys.stderr,
        )
        return 1

    print("\n".join(report(found)))
    if points_path is not None:
        with points_path.open("a") as handle:
            for point in points(found):
                handle.write(json.dumps(point) + "\n")
    return 0


def points(found: dict[str, float]) -> list[dict]:
    """Each expected row as a point for perf-gate.py to stamp and judge."""
    return [
        {
            "metric": f"slas.{key}",
            "statistic": "median",
            "unit": "ns",
            "value": found[key],
            # `cargo bench` builds with the bench profile, which is not the
            # debug profile the idle-RSS test runs under.
            "env": {"profile": "bench", "arch": platform.machine(), "renderer": "none"},
        }
        for key in ROWS
    ]


def selftest() -> int:
    """The report must refuse a run that measured nothing."""
    complete = {key: 1.0 for key in ROWS}
    assert report(complete), "a complete run reports"

    for key in ROWS:
        partial = {other: 1.0 for other in ROWS if other != key}
        try:
            report(partial)
        except KeyError:
            pass
        else:  # pragma: no cover - the failure this guards against
            print(f"selftest: a run missing {key} was reported anyway", file=sys.stderr)
            return 1

    over = dict.fromkeys(ROWS, 9e9)
    assert "❌" in "\n".join(report(over)), "a row over its SLA is marked"
    under = dict.fromkeys(ROWS, 1.0)
    assert "❌" not in "\n".join(report(under)), "a row under its SLA is not"

    emitted = points(dict.fromkeys(ROWS, 2.0))
    assert [point["metric"] for point in emitted] == [f"slas.{key}" for key in ROWS], (
        "every row becomes a point, named after its bench"
    )
    assert all(point["statistic"] == "median" for point in emitted), "points carry the median"

    print("selftest: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
