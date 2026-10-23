from __future__ import annotations

import json
from collections.abc import Mapping
from contextlib import suppress
from typing import Any


def content_tokens(value: Any, field: str = "") -> set[str]:
    tokens: set[str] = set()
    if isinstance(value, dict):
        for key, item in value.items():
            tokens.update(content_tokens(item, key))
    elif isinstance(value, list):
        for item in value:
            tokens.update(content_tokens(item, field))
    elif isinstance(value, str) and value:
        if value[:1] in "{[":
            with suppress(ValueError):
                tokens.update(content_tokens(json.loads(value), field))
        if any(part in field for part in ("sha", "hash", "patch_id", "image_oid", "commit_id")):
            tokens.add(value.lower())
    return tokens


def overlap(first: set[str], second: set[str]) -> bool:
    return any(a.startswith(b) or b.startswith(a) for a in first for b in second)


def plan_batches(tokens: Mapping[str, set[str]]) -> dict[str, int]:
    conflicts = {
        name: {other for other in tokens if other != name and overlap(values, tokens[other])}
        for name, values in tokens.items()
    }
    groups: list[set[str]] = []
    assignments: dict[str, int] = {}
    for name in sorted(tokens, key=lambda item: (-len(conflicts[item]), item)):
        for index, group in enumerate(groups):
            if not group.intersection(conflicts[name]):
                group.add(name)
                assignments[name] = index
                break
        else:
            assignments[name] = len(groups)
            groups.append({name})
    return assignments
