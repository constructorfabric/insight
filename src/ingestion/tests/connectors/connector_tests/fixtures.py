"""Fixture loading for connector mock suites.

Every suite keeps its response bodies in `tests/fixtures/*.json` — mandatory,
the same approach for big and small responses. Field names and shapes come from
real API payloads; every value is synthetic. Tests load a fixture and override
only the fields a case exercises.
"""

from __future__ import annotations

import copy
import json
from pathlib import Path
from typing import Any


def load_fixture(test_file: str, name: str, /, **overrides: Any) -> Any:
    """Parse `fixtures/<name>` next to the calling test module and apply
    top-level field overrides:

        record = load_fixture(__file__, "project.json", id="10002", key="PROJ2")

    The two parameters are positional-only so overrides may use any record
    field name (including `name`).
    """
    path = Path(test_file).resolve().parent / "fixtures" / name
    with open(path) as f:
        data = json.load(f)
    if overrides:
        if not isinstance(data, dict):
            raise TypeError(f"{name} is not a JSON object — cannot apply overrides")
        data = copy.deepcopy(data)
        data.update(overrides)
    return data
