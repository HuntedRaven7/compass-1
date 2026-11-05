#!/usr/bin/env python3
"""Report Rust source files under crates/ that are over the module size limit.

    python3 scripts/architecture/module-size.py
    python3 scripts/architecture/module-size.py --selftest

WHY THIS EXISTS

Compass keeps growing orchestration modules that own unrelated domains:
`compass-ui/src/app.rs` (#258), the engine's `EngineState` in `serve.rs`
(#292), and `compass-ipc/src/protocol.rs` (#294). Review finds them after they
have become expensive refactors. This makes the next one visible in the PR that
creates it (#330).

WHAT IT DOES NOT DO

It does not gate. The owner's decision on #330 is a warning-only report until
the existing refactors land: a file over the limit that is not in
`allowlist.txt` gets a GitHub `::warning` annotation and the exit status stays
0. It exits 1 only for its own failures -- an allowlist line it cannot read, or
a scan that found no Rust at all, which would otherwise look like a clean run.
"""

import contextlib
import io
import os
import pathlib
import re
import sys
import tempfile

LIMIT = 1500
HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent.parent
ALLOWLIST = HERE / "allowlist.txt"
ISSUE = re.compile(r"#\d+")


def scan(root: pathlib.Path) -> dict[str, int]:
    """Line counts of every `.rs` file under `root/crates`, keyed by repo-relative path."""
    sizes = {}
    for directory, subdirs, files in os.walk(root / "crates"):
        subdirs[:] = sorted(d for d in subdirs if d != "target" and not d.startswith("."))
        for name in sorted(files):
            if name.endswith(".rs"):
                path = pathlib.Path(directory, name)
                with path.open("rb") as file:
                    sizes[path.relative_to(root).as_posix()] = sum(1 for _ in file)
    return sizes


def load_allowlist(path: pathlib.Path) -> dict[str, str]:
    """`path #issue rationale` entries, keyed by path, valued by issue."""
    entries = {}
    for number, raw in enumerate(path.read_text().splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        fields = line.split(maxsplit=2)
        if len(fields) < 3 or not ISSUE.fullmatch(fields[1]):
            raise ValueError(
                f"{path}:{number}: expected `<path> #<issue> <rationale>`, got {line!r}"
            )
        entries[fields[0]] = fields[1]
    return entries


def report(sizes: dict[str, int], allowlist: dict[str, str], limit: int) -> list[str]:
    """The lines to print: a warning per unlisted offender, a notice per stale entry."""
    lines = []
    for path, count in sorted(sizes.items()):
        if count <= limit:
            continue
        if path in allowlist:
            lines.append(f"{path}: {count} lines, allowlisted ({allowlist[path]})")
        else:
            lines.append(
                f"::warning file={path},line=1,title=Module over {limit} lines::"
                f"{path} has {count} lines (limit {limit}). Split it, or add it to "
                "scripts/architecture/allowlist.txt with the issue tracking the split (#330)."
            )
    for path, issue in sorted(allowlist.items()):
        if sizes.get(path, 0) <= limit:
            lines.append(
                f"::notice title=Stale module-size allowlist entry::{path} ({issue}) is "
                f"missing or no longer over {limit} lines; remove it from the allowlist."
            )
    return lines


def check(root: pathlib.Path, allowlist_path: pathlib.Path, limit: int) -> int:
    """Print the report for `root`. Non-zero only when the check itself cannot run."""
    try:
        allowlist = load_allowlist(allowlist_path)
    except (OSError, ValueError) as error:
        print(f"::error::{error}")
        return 1
    sizes = scan(root)
    if not sizes:
        print(f"::error::no Rust files under {root / 'crates'} — the paths have moved")
        return 1
    lines = report(sizes, allowlist, limit)
    print("\n".join(lines))
    unlisted = sum(line.startswith("::warning") for line in lines)
    print(f"{len(sizes)} Rust file(s) scanned, {unlisted} over {limit} lines and not allowlisted")
    return 0


def selftest() -> int:
    """An oversized file must warn, an allowlisted one must not, and nothing must fail."""
    with tempfile.TemporaryDirectory() as tmp:
        root = pathlib.Path(tmp)
        src = root / "crates" / "synthetic" / "src"
        src.mkdir(parents=True)
        (src / "oversized.rs").write_text("//\n" * (LIMIT + 1))
        (src / "allowed.rs").write_text("//\n" * (LIMIT * 2))
        (src / "at_limit.rs").write_text("//\n" * LIMIT)
        allowlist = root / "allowlist.txt"
        allowlist.write_text(
            "# comment\n"
            "crates/synthetic/src/allowed.rs #1 synthetic\n"
            "crates/synthetic/src/at_limit.rs #2 under the limit, so stale\n"
        )

        lines = report(scan(root), load_allowlist(allowlist), LIMIT)
        warnings = [line for line in lines if line.startswith("::warning")]
        notices = [line for line in lines if line.startswith("::notice")]
        assert len(warnings) == 1, f"exactly one warning, got {warnings}"
        assert "crates/synthetic/src/oversized.rs has 1501 lines" in warnings[0], warnings
        assert not any("allowed.rs" in line for line in warnings), "allowlisted file warned"
        assert not any("at_limit.rs" in line for line in warnings), "file at the limit warned"
        assert len(notices) == 1 and "at_limit.rs" in notices[0], f"stale entry: {notices}"

        # Captured, so the synthetic annotations never reach a real run's summary.
        def quiet_check(scan_root: pathlib.Path, allowlist_path: pathlib.Path) -> int:
            with contextlib.redirect_stdout(io.StringIO()):
                return check(scan_root, allowlist_path, LIMIT)

        assert quiet_check(root, allowlist) == 0, "warnings alone must not fail the step"

        (root / "bad.txt").write_text("crates/synthetic/src/allowed.rs no-issue here\n")
        assert quiet_check(root, root / "bad.txt") == 1, "an entry without an issue"

        empty = root / "empty"
        (empty / "crates").mkdir(parents=True)
        assert quiet_check(empty, allowlist) == 1, "a scan that found nothing"

    print("selftest: ok")
    return 0


def main(argv: list[str]) -> int:
    if "--selftest" in argv:
        return selftest()
    return check(ROOT, ALLOWLIST, LIMIT)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
