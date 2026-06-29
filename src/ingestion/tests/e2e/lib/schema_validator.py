"""Per-table JSON schema: resolve by table name, pad, validate.

Implements the post-step of `cpt-bronze-to-api-e2e-algo-yaml-resolve-refs`
(DoD `cpt-bronze-to-api-e2e-dod-yaml-schema-resolution`). A schema lives in
`metrics/schemas/<table>.yaml` under a top-level `schemas: { <table>: {...} }`.
After `$ref` resolution a bronze record is padded with every missing schema
property as `null`, then validated; `additionalProperties:false` catches a
misspelled column name.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

import jsonschema
import yaml


class SchemaError(ValueError):
    """Missing schema file/entry, or a record that fails validation."""


_CACHE: dict[tuple[Path, str], dict] = {}


def load_schema(schemas_dir: Path, table: str) -> dict:
    """Load the JSON schema for `<table>` from `schemas/<table>.yaml`."""
    key = (schemas_dir.resolve(), table)
    if key not in _CACHE:
        path = schemas_dir / f"{table}.yaml"
        if not path.is_file():
            raise SchemaError(f"no schema for table '{table}': expected {path}")
        try:
            doc = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
        except yaml.YAMLError as e:
            raise SchemaError(f"{path}: invalid YAML: {e}") from e
        if not isinstance(doc, dict) or not isinstance(doc.get("schemas"), dict):
            raise SchemaError(f"{path}: must have a top-level `schemas:` mapping")
        schemas = doc["schemas"]
        if table not in schemas:
            raise SchemaError(f"{path}: missing `schemas.{table}` entry")
        _CACHE[key] = schemas[table]
    return _CACHE[key]


def pad_and_validate(record: dict, schema: dict, *, table: str) -> dict:
    """Pad `record` with missing schema properties (null), then validate it.

    Returns the padded record. Raises SchemaError on any violation.
    """
    props = schema.get("properties", {})
    padded = dict(record)
    for col in props:
        padded.setdefault(col, None)
    try:
        jsonschema.validate(padded, schema)
    except jsonschema.ValidationError as e:
        raise SchemaError(
            f"bronze record for '{table}' violates schema: {e.message} "
            f"(at {list(e.absolute_path) or '<root>'})"
        ) from e
    return padded
