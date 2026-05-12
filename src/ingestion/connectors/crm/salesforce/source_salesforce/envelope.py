"""Record envelope helpers.

Every record emitted to Bronze is augmented with tenant / source scope and a
deterministic ``unique_key`` so downstream dbt models can key off a single
stable identifier. Custom ``__c`` fields are pulled out into a single JSON
blob so the Bronze schema stays stable across orgs with different SF
customizations.
"""

import hashlib
import json
import logging
from datetime import datetime, timezone
from typing import Any, Mapping, MutableMapping, MutableSet, Optional

logger = logging.getLogger("airbyte")

DATA_SOURCE = "salesforce"

# Field names injected by the envelope. A real SF field that collides with one
# of these would otherwise be silently overwritten; we log and drop it instead.
_RESERVED_FIELD_NAMES = frozenset(
    {"tenant_id", "source_id", "unique_key", "data_source", "collected_at", "custom_fields"}
)


def _now_iso() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def envelope(
    record: Mapping[str, Any],
    *,
    tenant_id: str,
    source_id: str,
    custom_field_names: frozenset,
    collision_seen: Optional[MutableSet[str]] = None,
) -> MutableMapping[str, Any]:
    """Return a copy of ``record`` with Insight metadata injected.

    Splits the record: standard fields stay at top level, every name in
    ``custom_field_names`` is packed into a single ``custom_fields`` JSON string,
    and ``tenant_id`` / ``source_id`` / ``unique_key`` / ``data_source`` /
    ``collected_at`` are added.

    If ``collision_seen`` is provided, collision warnings are emitted only once
    per offending field name across the stream's lifetime.
    """
    out: MutableMapping[str, Any] = {}
    customs: dict = {}

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
        if key in custom_field_names:
            customs[key] = value
        else:
            out[key] = value

    # ClickHouse stores JSON blobs as strings; serialize once.
    out["custom_fields"] = (
        json.dumps(customs, separators=(",", ":"), default=str) if customs else "{}"
    )

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
    "custom_fields": {"type": "string"},
}


def inject_envelope_properties(schema: MutableMapping[str, Any]) -> MutableMapping[str, Any]:
    """Add envelope field definitions to a JSON schema generated from describe().

    Used when advertising per-stream schemas so the destination creates columns
    for the envelope fields alongside the SF fields.
    """
    props = schema.setdefault("properties", {})
    for name, spec in ENVELOPE_FIELDS_SCHEMA.items():
        props[name] = spec
    return schema
