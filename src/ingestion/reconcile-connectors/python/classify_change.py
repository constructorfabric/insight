#!/usr/bin/env python3
"""Classify a source-config change as 'noop' / 'non-breaking' / 'breaking'.

CLI: classify_change.py <current_json> <target_json>
Stdout: 'noop' | 'breaking' | 'non-breaking'.

Comparison rules:
  - Iterate only target's keys. Airbyte adds internal __injected_* fields
    on stored configs; ignoring extras keeps reconcile from chasing them.
  - Masked-secret values in `current` (a run of asterisks, which is what
    Airbyte returns for `airbyte_secret: true` fields on read) are treated
    as equal to anything — we cannot see the real stored value, so we
    cannot diff it. Rotation of a masked secret must therefore be detected
    via the cfg-hash tag (a SHA-256 of the K8s Secret data, which is what
    drives connection-tag drift in reconcile.sh) rather than via this
    config diff.
  - 'breaking' fires when a non-masked drifted key matches the
    re-tenanting pattern (host / database / schema / tenant /
    repository / etc.) — those need a state-preserving recreate.
  - 'non-breaking' fires when at least one non-masked, non-breaking
    field drifted.
  - 'noop' fires when no drift detected.
"""
import json
import re
import sys


_BREAKING_RE = re.compile(
    r"^(host|hostname|database|schema|catalog|account|workspace|tenant|orgId|organization|repository|stream)$",
    re.IGNORECASE,
)


def _is_masked(v) -> bool:
    """Airbyte returns `airbyte_secret: true` strings as a run of asterisks."""
    return isinstance(v, str) and len(v) > 0 and set(v) == {"*"}


def main() -> int:
    if len(sys.argv) != 3:
        sys.stderr.write("classify_change: expected 2 args (current, target)\n")
        return 2
    current = json.loads(sys.argv[1] or "{}")
    target = json.loads(sys.argv[2] or "{}")

    drift_breaking = False
    drift_nonbreaking = False
    for key, target_value in target.items():
        current_value = current.get(key)
        if _is_masked(current_value):
            continue
        if current_value == target_value:
            continue
        if _BREAKING_RE.match(key):
            drift_breaking = True
            break
        drift_nonbreaking = True

    if drift_breaking:
        print("breaking")
    elif drift_nonbreaking:
        print("non-breaking")
    else:
        print("noop")
    return 0


if __name__ == "__main__":
    sys.exit(main())
