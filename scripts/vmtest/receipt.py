#!/usr/bin/env python3
"""A versioned evidence receipt for the launcher's visual/harness gates.

This is Compass's local adapter, not the shared TunaOS checkpoint schema. It
wraps the producers we already have (the VM tier, the headless paint tier and
the fixture controls) so a result can be read as evidence: which source and
binary produced it, what was actually asserted, whether it was a gate or a
diagnostic, and whether the run cleaned up after itself.

The point is refusal, not bookkeeping. A receipt that is missing a required
assertion, that was only a diagnostic, that claims a portal activation while
recording CLI input, or that says "passed" for a skipped check must not be
read as a passed capability. See ``receipt-selftest.py`` for the negative
controls, run on every PR by ``shell.yaml``.
"""

from __future__ import annotations

import hashlib
from pathlib import Path

# Bump when the meaning of a field changes, not just when one is added.
SCHEMA_VERSION = 1

OUTCOMES = frozenset({"passed", "failed", "blocked", "not-run", "unsupported", "flaky"})
KINDS = frozenset({"gate", "diagnostic"})
INPUT_METHODS = frozenset({"portal", "keyboard", "pointer", "cli", "fixture"})
# CLI input and injected fixtures are not proof that a global shortcut arrived
# through the portal. Keep them distinct rather than discarding either.
REAL_INPUT_METHODS = frozenset({"portal", "keyboard", "pointer"})
EVIDENCE_CLASSES = frozenset({"fixture", "modified-canary", "published-image", "installed-media"})


def sha256_file(path: str | Path) -> str:
    """The hash of a file's bytes, for an artifact the receipt names."""
    digest = hashlib.sha256()
    with open(path, "rb") as stream:
        for chunk in iter(lambda: stream.read(65536), b""):
            digest.update(chunk)
    return digest.hexdigest()


def new_receipt(
    *,
    source_sha: str,
    harness_version: str,
    package: str,
    environment: str,
    image_digest: str,
    run: str,
    attempt: int,
    session: str,
    action: str,
    input_method: str,
    assertions: list[dict],
    artifacts: dict[str, dict[str, str]],
    cleanup_completed: bool,
    evidence_class: str = "fixture",
    claims_global_hotkey: bool = False,
    masks: list[dict[str, str]] | None = None,
    thresholds: dict[str, float] | None = None,
    diagnostic_only: bool = False,
) -> dict:
    """Build a receipt with the schema version and run identity filled in."""
    return {
        "schema": SCHEMA_VERSION,
        "source": {
            "sha": source_sha,
            "harness_version": harness_version,
            "schema_version": SCHEMA_VERSION,
        },
        "identity": {
            "package": package,
            "environment": environment,
            "image_digest": image_digest,
        },
        "run": {"id": run, "attempt": attempt, "session": session},
        "action": {
            "name": action,
            "input_method": input_method,
            "claims_global_hotkey": claims_global_hotkey,
        },
        "evidence_class": evidence_class,
        "assertions": assertions,
        "masks": masks or [],
        "thresholds": thresholds or {},
        "artifacts": artifacts,
        "diagnostic_only": diagnostic_only,
        "cleanup": {"completed": cleanup_completed},
    }


def rejections(
    receipt: dict,
    *,
    verify_artifacts: bool = False,
    expected: dict[str, str] | None = None,
) -> list[str]:
    """Every reason this receipt cannot be read as evidence, in a stable order.

    An empty list means the receipt is well-formed. ``qualifies`` is the
    stricter question of whether it also passed a capability gate. `expected`
    binds the receipt to the source, package, image and session the reader
    believes it is looking at; a mismatch is a refusal, not a warning.
    """
    bad: list[str] = []
    expected = expected or {}
    if receipt.get("schema") != SCHEMA_VERSION:
        bad.append(f"unsupported schema: {receipt.get('schema')!r}")
    source = receipt.get("source") or {}
    if source.get("schema_version") != SCHEMA_VERSION:
        bad.append("the source and receipt schema versions disagree")
    for key in ("sha", "harness_version"):
        if not source.get(key):
            bad.append(f"missing source.{key}")
    identity = receipt.get("identity") or {}
    for key in ("package", "environment", "image_digest"):
        if not identity.get(key):
            bad.append(f"missing identity.{key}")
    run = receipt.get("run") or {}
    for key in ("id", "session"):
        if not run.get(key):
            bad.append(f"missing run.{key}")
    if not isinstance(run.get("attempt"), int) or run["attempt"] < 1:
        bad.append("run.attempt must be a positive integer")
    action = receipt.get("action") or {}
    if not action.get("name"):
        bad.append("missing action.name")
    if action.get("input_method") not in INPUT_METHODS:
        bad.append(f"unknown input method: {action.get('input_method')!r}")
    if action.get("claims_global_hotkey") and action.get("input_method") not in REAL_INPUT_METHODS:
        bad.append("CLI or fixture input cannot claim a portal-delivered global hotkey")
    if receipt.get("evidence_class") not in EVIDENCE_CLASSES:
        bad.append(f"unknown evidence class: {receipt.get('evidence_class')!r}")

    assertions = receipt.get("assertions")
    if not isinstance(assertions, list) or not assertions:
        bad.append("no assertions")
    else:
        if not any(item.get("kind") == "gate" for item in assertions):
            bad.append("no gate assertion; diagnostics cannot qualify a capability")
        for item in assertions:
            name = item.get("name", "<unnamed>")
            if item.get("kind") not in KINDS:
                bad.append(f"{name}: unknown assertion kind")
            if item.get("outcome") not in OUTCOMES:
                bad.append(f"{name}: unknown outcome {item.get('outcome')!r}")
            if item.get("outcome") in {"blocked", "unsupported"} and not item.get("reason"):
                bad.append(f"{name}: {item.get('outcome')} needs a reason")

    for mask in receipt.get("masks") or []:
        if not mask.get("name") or not mask.get("reason"):
            bad.append("an ignored region has no justification")

    artifacts = receipt.get("artifacts") or {}
    if not artifacts:
        bad.append("no artifacts")
    for name, entry in artifacts.items():
        if not entry.get("sha256"):
            bad.append(f"artifact {name} has no hash")
            continue
        if verify_artifacts:
            path = Path(entry.get("path", ""))
            if not path.is_file():
                bad.append(f"artifact {name} is missing: {path}")
            elif sha256_file(path) != entry["sha256"]:
                bad.append(f"artifact {name} does not match its hash")

    if not isinstance(receipt.get("cleanup"), dict) or not receipt["cleanup"].get("completed"):
        bad.append("the receipt was finalized before cleanup")

    for key, value in expected.items():
        current = {
            "source.sha": source.get("sha"),
            "identity.package": identity.get("package"),
            "identity.image_digest": identity.get("image_digest"),
            "run.session": run.get("session"),
        }.get(key)
        if current != value:
            bad.append(f"{key} does not match the expected {value!r}")
    return bad


def qualifies(
    receipt: dict,
    *,
    verify_artifacts: bool = False,
    expected: dict[str, str] | None = None,
) -> bool:
    """Whether this receipt may be read as a passed capability.

    Every required assertion must have passed, the result must not be
    diagnostic-only, the evidence must not be a fixture standing in for a
    shipped system, and the receipt must be well-formed and cleaned up.
    """
    if rejections(receipt, verify_artifacts=verify_artifacts, expected=expected):
        return False
    if receipt["diagnostic_only"] or receipt["evidence_class"] == "fixture":
        return False
    required = [item for item in receipt["assertions"] if item["kind"] == "gate"]
    return bool(required) and all(item["outcome"] == "passed" for item in required)
