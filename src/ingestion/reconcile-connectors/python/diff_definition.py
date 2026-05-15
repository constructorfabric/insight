#!/usr/bin/env python3
"""Compare descriptor.yaml.version to remote definition declarativeManifest.description.

Per ADR-0015: target version is strict semver MAJOR.MINOR.PATCH. The remote
(current) value MAY be a legacy non-semver string for backward compatibility;
in that case the diff is classified as `migration` and downstream code treats
it like a `patch` bump (no full-refresh).

CLI:
  diff_definition.py --descriptor PATH --remote PATH

Stdout: a single TSV line `<action>\t<bump_kind>`, no trailing newline.
  action    ∈ {same, differ}
  bump_kind ∈ {none, patch, minor, major, migration}

Exit:   0 same (action=same, bump_kind=none)
        1 differ (action=differ, bump_kind=one of the four non-none kinds)
        2 error (e.g. target is not semver)

For nocode: remote-side comparison key is `definition.declarativeManifest.description`.
For CDK:   remote-side comparison key is `definition.dockerImageTag`.
The script auto-detects by presence of `declarativeManifest` in the remote JSON.
"""
import argparse
import json
import re
import sys

import yaml  # NB: PyYAML is a third-party dependency; install via project's requirements.


_SEMVER_RE = re.compile(r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$")


def _read_yaml_field(path: str, key: str):
    with open(path, "r", encoding="utf-8") as f:
        doc = yaml.safe_load(f)
    return doc.get(key)


def _parse_semver(value: str):
    m = _SEMVER_RE.match(value or "")
    if not m:
        return None
    return tuple(int(x) for x in m.groups())


def _classify(target: str, current: str) -> str:
    if target == current:
        return "none"
    t = _parse_semver(target)
    c = _parse_semver(current)
    if t is None:
        # Caller is expected to have validated target separately, but be defensive.
        return "major"
    if c is None:
        # Legacy non-semver current → first-time semver adoption for this connector.
        return "migration"
    if t[0] > c[0]:
        return "major"
    if t[0] == c[0] and t[1] > c[1]:
        return "minor"
    # Same major+minor and patch changed (forward or backward), or same major
    # and minor decreased: treat as patch — republish + re-discover, no
    # full-refresh. A "downgrade" still rolls the catalog forward but cannot
    # signal a destructive intent through the version field alone.
    return "patch"


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--descriptor", required=True)
    p.add_argument("--remote", required=True)
    args = p.parse_args()
    desired = _read_yaml_field(args.descriptor, "version")
    if desired is None:
        print("diff_definition: descriptor.version missing", file=sys.stderr)
        return 2
    desired = str(desired)
    if not _SEMVER_RE.match(desired):
        print(
            "diff_definition: descriptor.version "
            f"'{desired}' is not strict semver MAJOR.MINOR.PATCH (ADR-0015)",
            file=sys.stderr,
        )
        return 2
    with open(args.remote, "r", encoding="utf-8") as f:
        remote = json.load(f)
    if "declarativeManifest" in remote.get("definition", {}):
        actual = remote["definition"]["declarativeManifest"].get("description", "")
    else:
        actual = remote["definition"].get("dockerImageTag", "")
    actual = str(actual)
    bump_kind = _classify(desired, actual)
    action = "same" if bump_kind == "none" else "differ"
    sys.stdout.write(f"{action}\t{bump_kind}")
    return 0 if action == "same" else 1


if __name__ == "__main__":
    sys.exit(main())
