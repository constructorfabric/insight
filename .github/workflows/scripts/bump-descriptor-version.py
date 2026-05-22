#!/usr/bin/env python3
"""
bump-descriptor-version.py — bump the `version` field of a connector
descriptor.yaml by one minor increment per ADR-0015 (strict semver) and
ADR-0016 (descriptor.images: block).

Called by the CI `bump-descriptors` job and by `scripts/bootstrap-connector-
images.sh` whenever an image tag in `descriptor.yaml.images.<key>.image`
is updated. The minor bump makes reconcile re-discover the source catalog
on the next deploy (per ADR-0015 §catalog-refresh-on-bump) but stays below
the major-bump threshold that would dispatch a `dbt --full-refresh` — the
new image is a continuation of the same connector contract, not a breaking
change.

The script is **line-based**: it reads the file, finds the single
`^version:\\s*"?<X.Y.Z>"?\\s*$` line, replaces just that line with the
bumped value, and writes back. Comments, blank lines, and unrelated keys
are byte-identical across the rewrite — no YAML round-trip.

CLI:
    bump-descriptor-version.py --descriptor PATH [--print-only]

Exit:
    0  bumped (or `--print-only` printed; in both cases the new version
       is written to stdout)
    1  version field missing
    2  version is present but not strict semver MAJOR.MINOR.PATCH
       (per ADR-0015 — fail loud, do not auto-coerce)
"""
from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

# Strict semver: MAJOR.MINOR.PATCH where each segment is `0` or a
# non-zero digit followed by more digits — i.e. NO leading zeros (per
# semver.org §2). Date-style legacy values like `2026.05.04` are
# rejected because the `05` and `04` segments carry leading zeros;
# operators are expected to fix them to strict semver manually (per
# ADR-0015 §Version-format / migration tolerance). No `v` prefix, no
# pre-release suffix, no build metadata.
#
# The optional surrounding quotes match either `version: "1.2.3"` or
# `version: 1.2.3`; the rewrite preserves a double-quoted form (the
# common convention in our descriptors).
_SEMVER_SEGMENT = r'(?:0|[1-9]\d*)'
_VERSION_LINE_RE = re.compile(
    r'^(?P<indent>\s*)'
    r'version:\s*'
    r'(?P<quote>"?)'
    rf'(?P<major>{_SEMVER_SEGMENT})\.'
    rf'(?P<minor>{_SEMVER_SEGMENT})\.'
    rf'(?P<patch>{_SEMVER_SEGMENT})'
    r'(?P=quote)'
    r'\s*$',
    re.MULTILINE,
)


def bump_minor(descriptor_path: Path, print_only: bool = False) -> str:
    """Bump `version: X.Y.Z` → `X.(Y+1).0` in-place. Return the new version
    string. Raise SystemExit on missing field or non-semver value."""
    text = descriptor_path.read_text(encoding="utf-8")
    match = _VERSION_LINE_RE.search(text)
    if not match:
        # Differentiate the two failure cases: "version field totally missing"
        # vs "version present but not strict semver". The latter is the more
        # likely bug (legacy date-style values like 2026.05.04 still around).
        loose = re.search(r'^\s*version:\s*\S', text, re.MULTILINE)
        if loose is None:
            sys.stderr.write(
                f"ERROR: {descriptor_path}: no `version:` field. "
                f"Every descriptor MUST declare a strict-semver version per ADR-0015.\n"
            )
            sys.exit(1)
        else:
            current = loose.group(0).strip()
            sys.stderr.write(
                f"ERROR: {descriptor_path}: `version:` is not strict semver "
                f"MAJOR.MINOR.PATCH (per ADR-0015). Got: {current!r}.\n"
                f"       Fix it manually (e.g. set version: \"1.0.0\") before "
                f"this script can bump it.\n"
            )
            sys.exit(2)

    major = match.group("major")
    minor = int(match.group("minor"))
    new_version = f"{major}.{minor + 1}.0"
    new_line = f'{match.group("indent")}version: "{new_version}"'

    if not print_only:
        patched = text[: match.start()] + new_line + text[match.end():]
        descriptor_path.write_text(patched, encoding="utf-8")
    return new_version


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--descriptor", required=True,
                        help="path to the connector's descriptor.yaml")
    parser.add_argument("--print-only", action="store_true",
                        help="compute and print the new version without writing")
    args = parser.parse_args()

    descriptor = Path(args.descriptor)
    if not descriptor.is_file():
        sys.stderr.write(f"ERROR: {descriptor} not found\n")
        return 1

    new_version = bump_minor(descriptor, print_only=args.print_only)
    print(new_version)
    return 0


if __name__ == "__main__":
    sys.exit(main())
