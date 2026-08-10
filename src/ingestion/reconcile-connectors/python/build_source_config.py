#!/usr/bin/env python3
"""Build an Airbyte source connectionConfiguration from a K8s Secret payload.

A Secret can only carry strings, while connector specs declare booleans,
integers, numbers, arrays and objects. Airbyte json-schema-validates every
sources/create and sources/update call against the spec, so each value is
converted to its declared type here — before the request is built — and a
value that cannot be converted names its own field instead of coming back
as an opaque HTTP 422.

The platform-owned fields are added on top: identity (`insight_tenant_id`,
`insight_source_id`) from the tenant config and the Secret's
`insight.cyberfabric.com/source-id` annotation, plus anything passed as
`--injected` (the git-cli-proxy address and token), so an operator never has
to duplicate them into the Secret payload. They already carry their declared
types and bypass conversion.

CLI:
  kubectl get secret NAME -o json | extract_secret_data.py \
    | build_source_config.py --connector-dir DIR --connector-type TYPE \
        --tenant-id ID --source-id ID [--injected JSON]

Stdout: the connectionConfiguration JSON object on success, the failure
        reason otherwise — the caller logs it, since only its structured
        log line reaches the cluster's collector, and a reason on stderr
        would be discarded.
Exit:   0 built
        1 the connector's spec is missing or unreadable
        2 a value does not fit its declared type
"""

from __future__ import annotations

import argparse
import json
import math
import sys
from pathlib import Path

JsonValue = str | int | float | bool | None | list["JsonValue"] | dict[str, "JsonValue"]
JsonObject = dict[str, JsonValue]

_SCHEMA_TYPES = ("string", "number", "integer", "object", "array", "boolean", "null")


class SpecUnavailable(Exception):
    pass


class FieldRejected(Exception):
    def __init__(self, field: str, reason: str) -> None:
        super().__init__(f"{field}: {reason}")


def _load_connection_spec(connector_dir: Path, connector_type: str) -> JsonObject:
    if connector_type == "cdk":
        specs = sorted(connector_dir.glob("source_*/spec.json"))
        if len(specs) != 1:
            raise SpecUnavailable(f"expected exactly one source_*/spec.json under {connector_dir}, found {len(specs)}")
        try:
            spec = json.loads(specs[0].read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as e:
            raise SpecUnavailable(f"cannot read {specs[0]}: {e}") from e
        return spec.get("connectionSpecification") or {}

    # Only declarative connectors carry a manifest, so the cdk path above stays
    # runnable wherever PyYAML is absent.
    import yaml

    manifest_path = connector_dir / "connector.yaml"
    try:
        manifest = yaml.safe_load(manifest_path.read_text(encoding="utf-8")) or {}
    except (OSError, yaml.YAMLError) as e:
        raise SpecUnavailable(f"cannot read {manifest_path}: {e}") from e
    return (manifest.get("spec") or {}).get("connection_specification") or {}


def _parse_scalar(raw: str, declared: str) -> JsonValue:
    text = raw.strip()
    if declared == "boolean":
        lowered = text.lower()
        if lowered in ("true", "false"):
            return lowered == "true"
        raise ValueError('expected "true" or "false"')
    if declared == "integer":
        try:
            return int(text)
        except ValueError:
            raise ValueError("expected an integer") from None
    if declared == "number":
        try:
            parsed_number = float(text)
        except ValueError:
            raise ValueError("expected a number") from None
        if not math.isfinite(parsed_number):
            raise ValueError("expected a finite number") from None
        return parsed_number
    if declared in ("array", "object"):
        try:
            parsed = json.loads(raw)
        except json.JSONDecodeError:
            raise ValueError(f"expected a JSON {declared}") from None
        if not isinstance(parsed, list if declared == "array" else dict):
            raise ValueError(f"expected a JSON {declared}")
        return parsed
    if declared == "null":
        if text == "null":
            return None
        raise ValueError('expected "null"')
    return raw


def _matches(value: JsonValue, declared: str) -> bool:
    if declared == "boolean":
        return isinstance(value, bool)
    if declared == "integer":
        return isinstance(value, int) and not isinstance(value, bool)
    if declared == "number":
        return isinstance(value, int | float) and not isinstance(value, bool)
    if declared == "array":
        return isinstance(value, list)
    if declared == "object":
        return isinstance(value, dict)
    if declared == "null":
        return value is None
    return isinstance(value, str)


def coerce_field(field: str, value: JsonValue, declared: JsonValue) -> JsonValue:
    """Convert one Secret value to the type its spec property declares.

    `declared` follows JSON Schema: a type name, a list of accepted type
    names, or None for a property the spec does not describe (passed
    through untouched, as Airbyte only validates what it declares).
    """
    types = [t for t in (declared if isinstance(declared, list) else [declared]) if isinstance(t, str)]
    if not types:
        return value

    unsupported = [t for t in types if t not in _SCHEMA_TYPES]
    if unsupported:
        raise FieldRejected(field, f"spec declares unsupported type {unsupported[0]!r}")

    if not isinstance(value, str):
        if any(_matches(value, t) for t in types):
            return value
        raise FieldRejected(field, f"expected {' or '.join(types)}, got {type(value).__name__}")

    if "string" not in types and not value.strip():
        raise FieldRejected(field, f"empty value — omit the key to leave {' or '.join(types)} unset")

    reasons: list[str] = []
    for declared_type in types:
        try:
            return _parse_scalar(value, declared_type)
        except ValueError as e:
            reasons.append(str(e))

    raise FieldRejected(field, "; ".join(dict.fromkeys(reasons)))


def build_config(
    secret_data: JsonObject,
    spec: JsonObject,
    tenant_id: str,
    source_id: str,
    injected: JsonObject | None = None,
) -> JsonObject:
    properties = spec.get("properties") or {}
    config = {
        key: coerce_field(key, value, (properties.get(key) or {}).get("type")) for key, value in secret_data.items()
    }
    config.update(injected or {})
    config["insight_tenant_id"] = tenant_id
    config["insight_source_id"] = source_id
    return config


def _fail(reason: str, code: int) -> int:
    sys.stdout.write(reason)
    return code


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--connector-dir", required=True)
    p.add_argument("--connector-type", required=True)
    p.add_argument("--tenant-id", required=True)
    p.add_argument("--source-id", required=True)
    p.add_argument("--injected", default="{}")
    args = p.parse_args()

    try:
        secret_data = json.loads(sys.stdin.read() or "{}") or {}
    except json.JSONDecodeError as e:
        return _fail(f"bad JSON on stdin: {e}", 1)
    if not isinstance(secret_data, dict):
        return _fail(f"expected a JSON object on stdin, got {type(secret_data).__name__}", 1)

    try:
        injected = json.loads(args.injected or "{}") or {}
    except json.JSONDecodeError as e:
        return _fail(f"bad JSON in --injected: {e}", 1)
    if not isinstance(injected, dict):
        return _fail(f"expected a JSON object in --injected, got {type(injected).__name__}", 1)

    try:
        spec = _load_connection_spec(Path(args.connector_dir), args.connector_type)
    except SpecUnavailable as e:
        return _fail(str(e), 1)

    try:
        config = build_config(secret_data, spec, args.tenant_id, args.source_id, injected)
    except FieldRejected as e:
        return _fail(str(e), 2)

    json.dump(config, sys.stdout)
    return 0


if __name__ == "__main__":
    sys.exit(main())
