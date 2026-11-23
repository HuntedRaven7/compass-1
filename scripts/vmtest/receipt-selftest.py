#!/usr/bin/env python3
"""Positive and deliberate negative controls for the visual receipt adapter.

Run directly; prints each case and exits non-zero if any behaves wrongly.
``shell.yaml`` runs this on every PR, alongside the frame-diff and launcher
selftests. Every negative control mutates one thing in the *valid* receipt and
must be refused through the same ``qualifies`` path a real capability uses.
"""

from __future__ import annotations

import copy
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import receipt  # noqa: E402

PASS = {"name": "launcher-paints-in-window", "kind": "gate", "outcome": "passed"}
DIAGNOSTIC = {"name": "screenshot-saved", "kind": "diagnostic", "outcome": "passed"}


def set_path(target: dict, dotted: str, value) -> None:
    """Set `a.b.c` in a nested receipt dict."""
    parts = dotted.split(".")
    for part in parts[:-1]:
        target = target[part]
    target[parts[-1]] = value


def valid(**overrides) -> dict:
    receipt_dict = receipt.new_receipt(
        source_sha="399210cba40a0a55445623b920317ade25d18fc2",
        harness_version="vmtest-4",
        package="org.tunaos.compass//stable",
        environment="bluefin-44-gnome-50",
        image_digest="sha256:abc123",
        run="run-1",
        attempt=1,
        session="session-1",
        action="summon",
        input_method="portal",
        assertions=[copy.deepcopy(PASS), copy.deepcopy(DIAGNOSTIC)],
        artifacts={"frame": {"path": "/evidence/frame.png", "sha256": "012345"}},
        cleanup_completed=True,
        evidence_class="installed-media",
        claims_global_hotkey=True,
        masks=[{"name": "top-bar", "reason": "GNOME clock changes every minute"}],
        thresholds={"min-changed-percent": "3"},
    )
    for key, value in overrides.items():
        set_path(receipt_dict, key, value)
    return receipt_dict


CASES: list[tuple[str, bool, dict]] = [
    ("valid installed-media portal receipt", True, valid()),
    ("fixture cannot qualify a shipped capability", False, valid(evidence_class="fixture")),
    ("diagnostic-only cannot qualify", False, valid(diagnostic_only=True)),
    (
        "a skipped-as-passed gate cannot qualify",
        False,
        valid(assertions=[{"name": "launcher-paints-in-window", "kind": "gate", "outcome": "not-run"}, DIAGNOSTIC]),
    ),
    (
        "a blocked gate cannot qualify",
        False,
        valid(assertions=[{"name": "launcher-paints-in-window", "kind": "gate", "outcome": "blocked", "reason": "no compositor"}, DIAGNOSTIC]),
    ),
    (
        "a gate missing entirely cannot qualify",
        False,
        valid(assertions=[DIAGNOSTIC]),
    ),
    (
        "CLI input cannot claim a portal-delivered hotkey",
        False,
        valid(**{"action.input_method": "cli"}),
    ),
    (
        "fixture input cannot claim a portal-delivered hotkey",
        False,
        valid(**{"action.input_method": "fixture"}),
    ),
    ("an unsupported schema is refused", False, valid(schema=999)),
    (
        "source and receipt schema versions must agree",
        False,
        valid(**{"source.schema_version": 999}),
    ),
    (
        "mismatched source identity is refused",
        False,
        valid(**{"source.sha": "a-different-commit"}),
    ),
    (
        "mismatched session identity is refused",
        False,
        valid(**{"run.session": "another-session"}),
    ),
    ("a missing artifact is refused", False, valid(artifacts={})),
    ("an unhashed artifact is refused", False, valid(artifacts={"frame": {"path": "/evidence/frame.png"}})),
    ("finalizing before cleanup is refused", False, valid(cleanup=False)),
    (
        "an unjustified ignored region is refused",
        False,
        valid(masks=[{"name": "top-bar"}]),
    ),
    (
        "an unknown outcome is refused",
        False,
        valid(assertions=[{"name": "launcher-paints-in-window", "kind": "gate", "outcome": "green"}, DIAGNOSTIC]),
    ),
]


def check_hash_verification() -> bool:
    """The hash check must read the artifact, not trust the producer's word."""
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "frame.png"
        path.write_bytes(b"a painted frame")
        good = receipt.new_receipt(
            source_sha="abc", harness_version="vmtest-4", package="org.tunaos.compass",
            environment="gnome-50", image_digest="sha256:abc", run="r", attempt=1,
            session="s", action="summon", input_method="portal", assertions=[PASS],
            artifacts={"frame": {"path": str(path), "sha256": receipt.sha256_file(path)}},
            cleanup_completed=True, evidence_class="installed-media",
        )
        assert receipt.qualifies(good, verify_artifacts=True)
        tampered = copy.deepcopy(good)
        tampered["artifacts"]["frame"]["sha256"] = "not-the-hash"
        fails = receipt.rejections(tampered, verify_artifacts=True)
        unchecked = receipt.rejections(tampered, verify_artifacts=False)
        return (
            any("does not match its hash" in line for line in fails)
            and not unchecked
        )


def main() -> int:
    # The reader's view of the revision, package and session. A receipt from
    # another build cannot qualify against this binding.
    bound = {
        "source.sha": "399210cba40a0a55445623b920317ade25d18fc2",
        "identity.package": "org.tunaos.compass//stable",
        "run.session": "session-1",
    }
    failures = 0
    for name, expected, receipt_dict in CASES:
        got = receipt.qualifies(receipt_dict, expected=bound)
        okay = got == expected
        print(f"{'ok  ' if okay else 'FAIL'} {name}: expected {expected}, got {got}")
        if not okay:
            print(f"     rejections: {receipt.rejections(receipt_dict, expected=bound)}")
            failures += 1
    okay = check_hash_verification()
    print(f"{'ok  ' if okay else 'FAIL'} artifact hashes are read from the file")
    if not okay:
        failures += 1
    print(f"{len(CASES) + 1} cases, {failures} failures")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
