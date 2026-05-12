#!/usr/bin/env python3
"""Builder-UI strict validator for declarative Airbyte manifests.

Runs the exact jsonschema check the Airbyte Builder UI performs — no `$ref`
resolution — and emits per-path errors with the deepest-matching `oneOf` branch
context so the user can pinpoint bad fields. Exits non-zero on any error.

Designed to run inside the `airbyte/source-declarative-manifest` Docker image,
which ships the `airbyte_cdk` package, `pyyaml`, and `jsonschema`. The schema
path is resolved dynamically from `airbyte_cdk.__file__` so the validator
survives Python-version bumps in the upstream image.

Usage:
    python3 validate_strict.py <manifest.yaml>
"""

from __future__ import annotations

import sys
from pathlib import Path

import airbyte_cdk
import jsonschema
import yaml


def _resolve_schema_path() -> Path:
    return (
        Path(airbyte_cdk.__file__).resolve().parent
        / "sources"
        / "declarative"
        / "declarative_component_schema.yaml"
    )


def _best_leaf(error: jsonschema.exceptions.ValidationError) -> jsonschema.exceptions.ValidationError:
    """Pick the deepest, most-informative leaf error from a oneOf union.

    `jsonschema.iter_errors` returns top-level errors that bundle every branch
    of a `oneOf` union as "context". The top-level message is usually noise
    ("is not valid under any of the given schemas"); the actual root cause is
    in the deepest non-"is not one of" leaf.
    """
    leaves: list[jsonschema.exceptions.ValidationError] = []

    def walk(cur: jsonschema.exceptions.ValidationError) -> None:
        if cur.context:
            for ce in cur.context:
                walk(ce)
        else:
            leaves.append(cur)

    walk(error)
    if not leaves:
        return error

    def score(c: jsonschema.exceptions.ValidationError) -> tuple[int, int]:
        path_depth = len(list(c.absolute_path))
        not_noise = "is not one of" not in c.message
        return (1 if not_noise else 0, path_depth)

    return sorted(leaves, key=score, reverse=True)[0]


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print(f"usage: {argv[0]} <manifest.yaml>", file=sys.stderr)
        return 2

    manifest_path = Path(argv[1])
    if not manifest_path.is_file():
        print(f"ERROR: manifest not found: {manifest_path}", file=sys.stderr)
        return 2

    schema_path = _resolve_schema_path()
    with schema_path.open() as f:
        schema = yaml.safe_load(f)
    with manifest_path.open() as f:
        manifest = yaml.safe_load(f)

    validator = jsonschema.Draft7Validator(schema)
    errors = list(validator.iter_errors(manifest))
    if not errors:
        print("Manifest is strictly valid (Builder-UI compatible)")
        return 0

    print(f"STRICT VALIDATION FAILED — {len(errors)} top-level error(s)")
    for i, error in enumerate(errors, 1):
        leaf = _best_leaf(error)
        path = "/".join(str(p) for p in leaf.absolute_path)
        print(f"  [{i}] {path}: {leaf.message[:240]}")
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
