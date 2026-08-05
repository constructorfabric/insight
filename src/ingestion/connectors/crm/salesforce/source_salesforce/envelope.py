"""Record envelope helpers.

Every record emitted to Bronze is augmented with tenant / source scope and a
deterministic ``unique_key`` so downstream dbt models can key off a single
stable identifier. Columns are limited to the stream's declared fields so the
Bronze schema is identical across orgs; the unabridged record is preserved in
``raw_data``.
"""

import hashlib
import json
import logging
from datetime import datetime, timezone
from typing import Any, FrozenSet, Mapping, MutableMapping, MutableSet, Optional

logger = logging.getLogger("airbyte")

DATA_SOURCE = "salesforce"

# Field names injected by the envelope. A real SF field that collides with one
# of these would otherwise be silently overwritten; we log and drop it instead.
_RESERVED_FIELD_NAMES = frozenset(
    {"tenant_id", "source_id", "unique_key", "data_source", "collected_at", "raw_data"}
)

# Per-value string cap inside ``raw_data``. Bounds worst-case row width however
# an org customizes its objects — long-text and rich-text fields are otherwise
# unbounded. The suffix keeps a clipped value distinguishable from a whole one.
_VALUE_MAX_BYTES = 2048
_TRUNCATED_SUFFIX = "…[truncated]"
_TRUNCATED_SUFFIX_BYTES = _TRUNCATED_SUFFIX.encode("utf-8")


def _truncate(value: Any) -> Any:
    if not isinstance(value, str):
        return value

    encoded = value.encode("utf-8")
    if len(encoded) <= _VALUE_MAX_BYTES:
        return value

    allowed = _VALUE_MAX_BYTES - len(_TRUNCATED_SUFFIX_BYTES)
    if allowed <= 0:
        return _TRUNCATED_SUFFIX
    # Slice on bytes; ``errors="ignore"`` drops a partial multi-byte char at the
    # boundary so the result stays valid UTF-8.
    return encoded[:allowed].decode("utf-8", errors="ignore") + _TRUNCATED_SUFFIX


def _truncated_copy(value: Any) -> Any:
    if isinstance(value, Mapping):
        return {k: _truncated_copy(v) for k, v in value.items()}
    if isinstance(value, list):
        return [_truncated_copy(item) for item in value]
    return _truncate(value)


def _now_iso() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def envelope(
    record: Mapping[str, Any],
    *,
    tenant_id: str,
    source_id: str,
    declared_fields: FrozenSet[str],
    collision_seen: Optional[MutableSet[str]] = None,
) -> MutableMapping[str, Any]:
    """Return a copy of ``record`` with Insight metadata injected.

    Fields in ``declared_fields`` stay at top level, the whole record is
    preserved in ``raw_data``, and ``tenant_id`` / ``source_id`` /
    ``unique_key`` / ``data_source`` / ``collected_at`` are added. A field the
    stream does not declare — a custom ``__c`` field, or a standard field
    outside the stream's schema — reaches Bronze through ``raw_data`` alone;
    emitting it top-level would make the table shape follow the org's field set.

    If ``collision_seen`` is provided, collision warnings are emitted only once
    per offending field name across the stream's lifetime.
    """
    out: MutableMapping[str, Any] = {}
    raw: dict = {}

    for key, value in record.items():
        # Salesforce always returns an ``attributes`` metadata dict — drop it.
        if key == "attributes":
            continue
        if key in _RESERVED_FIELD_NAMES:
            if collision_seen is None or key not in collision_seen:
                logger.warning(
                    "SF field %r collides with Insight envelope field; original value dropped",
                    key,
                )
                if collision_seen is not None:
                    collision_seen.add(key)
            continue

        raw[key] = _truncated_copy(value)
        if key in declared_fields:
            out[key] = value

    # ClickHouse stores JSON blobs as strings; serialize once.
    out["raw_data"] = json.dumps(raw, separators=(",", ":"), default=str) if raw else "{}"

    sf_id = record.get("Id") or record.get("id") or ""
    if not sf_id:
        # Every SF sobject we sync has an Id; an empty Id means either a
        # malformed response or a query shape (aggregate / GROUP BY) we
        # shouldn't be running. Bronze uses ReplacingMergeTree(_version)
        # ordered by unique_key with allow_nullable_key=1 — NULL keys would
        # still collide and collapse on merge. Derive a stable content hash
        # so malformed rows stay distinct across the merge.
        logger.error(
            "SF record missing Id; unique_key derived from content hash (tenant=%s source=%s record_keys=%s)",
            tenant_id,
            source_id,
            list(record.keys())[:10],
        )
        canonical = json.dumps(record, sort_keys=True, default=str)
        sf_id = f"nohash:{hashlib.sha256(canonical.encode('utf-8')).hexdigest()[:16]}"

    out["tenant_id"] = tenant_id
    out["source_id"] = source_id
    out["unique_key"] = f"{tenant_id}-{source_id}-{sf_id}"
    out["data_source"] = DATA_SOURCE
    out["collected_at"] = _now_iso()
    return out


ENVELOPE_FIELDS_SCHEMA = {
    "tenant_id": {"type": "string"},
    "source_id": {"type": "string"},
    "unique_key": {"type": "string"},
    "data_source": {"type": "string"},
    "collected_at": {"type": "string", "format": "date-time"},
    "raw_data": {"type": "string"},
}


def inject_envelope_properties(schema: MutableMapping[str, Any]) -> MutableMapping[str, Any]:
    """Add envelope field definitions to a stream's JSON schema.

    Used when advertising per-stream schemas so the destination creates columns
    for the envelope fields alongside the SF fields.
    """
    props = schema.setdefault("properties", {})
    for name, spec in ENVELOPE_FIELDS_SCHEMA.items():
        props[name] = spec
    return schema
