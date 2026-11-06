#!/usr/bin/env python3
"""Decides whether main has shipped anything since the last release, and how
big a version bump that is.

auto-release.yaml runs this daily. A release is due when a file that ends up
in what users install changed since the tag manifest.yaml names: engine and
UI code, the extension runtime, the bundled extensions, the packaging. Tests,
benches, docs, CI and the harnesses do not make a release on their own, and
neither does the Homebrew checksum PR a release itself opens (otherwise every
release would schedule the next).

The bump is `minor` when a commit in the range says it adds a feature
(`feat:`/`feat(...)` or `!:`/`BREAKING CHANGE`, the Conventional Commits
markers), else `patch`. A dispatch may ask for a level outright.

Prints `bump=<patch|minor|major|none>` and `reason=<sentence>`, also to
$GITHUB_OUTPUT when set. `--selftest` checks the rules without git.
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys

# Paths whose changes reach what users install.
SHIPPED_PREFIXES = (
    "crates/",
    "src/typescript/",
    "extensions/",
    "figura/",
    "packaging/",
    "extra/",
)
SHIPPED_FILES = ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml")

# Inside those, what never ships.
NOT_SHIPPED = (
    re.compile(r"^crates/[^/]+/(tests|benches|examples)/"),
    re.compile(r"\.md$"),
    re.compile(r"^packaging/vmtest/"),
    # The checksum PR a release opens: shipping it would schedule the next.
    re.compile(r"^packaging/homebrew/"),
)

FEATURE = re.compile(r"^feat(\([^)]*\))?!?:")
BREAKING = re.compile(r"^[a-z]+(\([^)]*\))?!:|BREAKING[ -]CHANGE", re.MULTILINE)

LEVELS = ("patch", "minor", "major")


def shipped(paths: list[str]) -> list[str]:
    """The paths in `paths` that reach what users install."""
    return [
        path
        for path in paths
        if (path.startswith(SHIPPED_PREFIXES) or path in SHIPPED_FILES)
        and not any(rule.search(path) for rule in NOT_SHIPPED)
    ]


def level(messages: list[str], requested: str) -> str:
    """The bump for commits with these full messages. Before 1.0 a breaking
    change is a minor bump, as SemVer's 0.y.z allows."""
    if requested in LEVELS:
        return requested
    if any(FEATURE.search(message) or BREAKING.search(message) for message in messages):
        return "minor"
    return "patch"


def git(*args: str) -> str:
    return subprocess.run(
        ["git", *args], check=True, capture_output=True, text=True
    ).stdout


def manifest_tag(path: str = "manifest.yaml") -> str:
    with open(path, encoding="utf-8") as manifest:
        for line in manifest:
            found = re.match(r'^  tag: "(v[0-9.]+)"$', line.rstrip("\n"))
            if found:
                return found.group(1)
    raise SystemExit(f"{path} names no release tag")


def decide(requested: str) -> tuple[str, str]:
    tag = manifest_tag()
    if subprocess.run(["git", "rev-parse", "--verify", "--quiet", f"{tag}^{{commit}}"],
                      capture_output=True).returncode != 0:
        return "none", f"{tag}, the last release manifest.yaml names, is not tagged yet"
    changed = shipped(git("diff", "--name-only", f"{tag}..HEAD").split())
    if not changed and requested not in LEVELS:
        return "none", f"nothing that ships changed since {tag}"
    messages = git("log", "--format=%B%x00", f"{tag}..HEAD").split("\x00")
    bump = level([message.strip() for message in messages if message.strip()], requested)
    return bump, f"{len(changed)} shipped file(s) changed since {tag}"


def selftest() -> None:
    checks = [
        (shipped(["crates/compass/src/serve.rs"]) == ["crates/compass/src/serve.rs"], "engine code ships"),
        (shipped(["crates/compass/tests/cli.rs"]) == [], "tests do not"),
        (shipped(["crates/compass-ui/benches/x.rs", "crates/a/examples/b.rs"]) == [], "benches and examples do not"),
        (shipped(["docs/PLAN.md", "crates/compass/README.md"]) == [], "docs do not"),
        (shipped([".github/workflows/rust.yaml", "scripts/ci/perf-gate.py"]) == [], "CI does not"),
        (shipped(["packaging/homebrew/compass.rb"]) == [], "the release's own checksum PR does not"),
        (shipped(["packaging/vmtest/checks.sh"]) == [], "the VM harness does not"),
        (shipped(["packaging/flatpak/org.tunaos.compass.yml"]) != [], "the Flatpak manifest does"),
        (shipped(["Cargo.lock"]) == ["Cargo.lock"], "a dependency update does"),
        (shipped(["src/typescript/api/src/index.ts"]) != [], "the extension runtime does"),
        (level(["fix: a thing"], "auto") == "patch", "a fix is a patch"),
        (level(["First run: ask for the hotkey"], "auto") == "patch", "an unmarked commit is a patch"),
        (level(["fix: a", "feat(ui): b"], "auto") == "minor", "a feature is a minor"),
        (level(["refactor!: drop x"], "auto") == "minor", "a breaking change before 1.0 is a minor"),
        (level(["chore: x\n\nBREAKING CHANGE: y"], "auto") == "minor", "a breaking footer too"),
        (level(["fix: a"], "major") == "major", "a requested level wins"),
        (level(["featurette: no"], "auto") == "patch", "only the feat marker counts"),
    ]
    failed = [name for ok, name in checks if not ok]
    for name in failed:
        print(f"FAIL: {name}", file=sys.stderr)
    if failed:
        raise SystemExit(1)
    print(f"selftest: {len(checks)} checks ok")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--bump", default="auto", choices=("auto", *LEVELS))
    parser.add_argument("--selftest", action="store_true")
    args = parser.parse_args()
    if args.selftest:
        selftest()
        return
    bump, reason = decide(args.bump)
    lines = [f"bump={bump}", f"reason={reason}"]
    print("\n".join(lines))
    output = os.environ.get("GITHUB_OUTPUT")
    if output:
        with open(output, "a", encoding="utf-8") as out:
            out.write("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
