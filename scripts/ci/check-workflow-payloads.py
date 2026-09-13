#!/usr/bin/env python3
"""Parse the shell payloads embedded inside workflow YAML.

WHY THIS EXISTS

Several workflows run a script inside a container as a single-quoted argument:

    podman run ... bash -euxo pipefail -c '
      ...forty lines of shell...
    '

YAML parsers are happy with anything in there, and `bash -n` on the step's
`run:` block only catches a broken payload by accident, because the payload is
just a string to the outer shell until the quoting goes wrong. Two defects of
exactly this shape have already reached a branch in this project:

  * an inner heredoc whose terminator matched the outer one, and
  * the word "Fedora's" in a comment, whose apostrophe closed the payload.

Both are invisible to YAML validation and cost a CI round to find. This finds
them in about a second.

WHAT IT CHECKS

Every `run:` block parses under `bash -n`, and so does every single-quoted
`bash ... -c '...'` payload found inside one. It also insists that at least one
payload is found overall: a regex that silently stops matching would otherwise
turn this into a check that passes by doing nothing, which is the failure mode
this repository keeps tripping over.
"""

import re
import subprocess
import sys
import tempfile
from pathlib import Path

import yaml

PAYLOAD = re.compile(r"bash [^\n]*-c '\n(.*?)\n\s*'", re.S)


def parses(source: str) -> tuple[bool, str]:
    with tempfile.NamedTemporaryFile("w", suffix=".sh", delete=False) as handle:
        handle.write(source)
        path = handle.name
    try:
        done = subprocess.run(["bash", "-n", path], capture_output=True, text=True)
        return done.returncode == 0, done.stderr.strip()
    finally:
        Path(path).unlink(missing_ok=True)


def main() -> int:
    workflows = sorted(Path(".github/workflows").glob("*.y*ml"))
    if not workflows:
        print("::error::no workflows found — the path has moved")
        return 1

    status = 0
    payloads = 0
    steps = 0

    for path in workflows:
        try:
            doc = yaml.safe_load(path.read_text())
        except yaml.YAMLError as err:
            print(f"::error file={path}::YAML does not parse: {err}")
            status = 1
            continue

        for job in (doc or {}).get("jobs", {}).values():
            for step in job.get("steps", []) or []:
                run = step.get("run")
                if not run:
                    continue
                steps += 1
                name = step.get("name", "(unnamed)")

                ok, err = parses(run)
                if not ok:
                    print(f"::error file={path}::step {name!r} does not parse: {err}")
                    status = 1

                for match in PAYLOAD.finditer(run):
                    payloads += 1
                    ok, err = parses(match.group(1))
                    if not ok:
                        print(
                            f"::error file={path}::the container payload in step "
                            f"{name!r} does not parse: {err}"
                        )
                        status = 1

    print(f"checked {steps} run blocks and {payloads} embedded payloads")

    # A regex that stops matching would make this pass by checking nothing.
    if payloads == 0:
        print("::error::found no embedded payloads at all — the pattern has rotted")
        return 1

    return status


if __name__ == "__main__":
    sys.exit(main())
