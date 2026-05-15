#!/usr/bin/env python3
"""Extract leading column from descriptor TSV and emit JSON list of names.

Stdin:  rows emitted by `disc_load_descriptors` (any number of TSV fields;
        only the first column `name` is read).
Stdout: JSON array of name strings (preserves order; skips empty).
Exit:   0 always.
"""
import json
import sys
from typing import List


def main() -> int:
    names: List[str] = []
    for line in sys.stdin:
        parts = line.rstrip("\n").split("\t")
        if parts and parts[0]:
            names.append(parts[0])
    print(json.dumps(names))
    return 0


if __name__ == "__main__":
    sys.exit(main())
