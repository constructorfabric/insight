#!/usr/bin/env python3
"""Classify a version bump as none|patch|minor|major|migration.

Per ADR-0015:
  - target MUST be strict semver MAJOR.MINOR.PATCH.
  - current MAY be a legacy non-semver string (e.g. "2026.05.04", "1.0").
  - none      → strings equal.
  - migration → current is not semver and target is semver and they differ.
  - patch     → both semver, major==major, minor==minor.
  - minor     → both semver, major==major, minor increased.
  - major     → both semver, major increased.

CLI:
  classify_bump.py <target> <current>

Stdout: one of {none, patch, minor, major, migration}, no trailing newline.
Exit:   0 on valid target (regardless of bump kind).
        2 if target is not strict semver.
"""
import re
import sys


# Strict semver per semver.org §2: numeric identifiers MUST NOT include
# leading zeroes. This deliberately excludes date-shaped values like
# `2026.05.04`, which would otherwise pass a naive `\d+\.\d+\.\d+`. The
# whole point of the leading-zero ban is to make semver and zero-padded
# dates lexically distinguishable.
_SEMVER_RE = re.compile(r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$")


def _parse_semver(value: str):
    m = _SEMVER_RE.match(value or "")
    if not m:
        return None
    return tuple(int(x) for x in m.groups())


def classify(target: str, current: str) -> str:
    if target == current:
        return "none"
    c = _parse_semver(current)
    t = _parse_semver(target)
    # t is guaranteed non-None by caller — main() validates before calling.
    if c is None:
        return "migration"
    if t[0] > c[0]:
        return "major"
    if t[0] == c[0] and t[1] > c[1]:
        return "minor"
    return "patch"


def main(argv) -> int:
    if len(argv) != 3:
        print("usage: classify_bump.py <target> <current>", file=sys.stderr)
        return 2
    target, current = argv[1], argv[2]
    if not _SEMVER_RE.match(target):
        print(
            f"classify_bump: target '{target}' is not strict semver "
            "MAJOR.MINOR.PATCH (ADR-0015)",
            file=sys.stderr,
        )
        return 2
    sys.stdout.write(classify(target, current))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
