"""Record envelope helpers for HubSpot records.

Every record emitted to Bronze is augmented with tenant / source scope and a
deterministic ``unique_key`` so downstream dbt models can key off a single
stable identifier. HubSpot custom properties (``hubspotDefined=false``) are
pulled out into a single JSON blob so the Bronze schema stays stable across
portals with different customizations.
"""

import hashlib
import json
import logging
from datetime import datetime, timezone
from typing import Any, Mapping, MutableMapping, MutableSet, Optional

logger = logging.getLogger("airbyte")

DATA_SOURCE = "hubspot"

# Field names injected by the envelope. A HubSpot property that collides
# (unlikely but possible with custom flat-top properties) would otherwise be
# silently overwritten; we log and drop it instead.
_RESERVED_FIELD_NAMES = frozenset(
    {"tenant_id", "source_id", "unique_key", "data_source", "collected_at", "custom_fields"}
)

# Per-property string truncation cap. Bounds the worst-case row width
# regardless of how a portal customizes its CRM (some custom properties hold
# multi-KB blobs of free-text or URL lists). Truncated suffix preserves the
# signal that a value was clipped so downstream isn't fooled into thinking
# it's the full content.
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
    # Slice on bytes; ``errors="ignore"`` drops a partial multi-byte char at
    # the boundary so the result remains valid UTF-8.
    return encoded[:allowed].decode("utf-8", errors="ignore") + _TRUNCATED_SUFFIX


def _now_iso() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def envelope(
    record: Mapping[str, Any],
    *,
    tenant_id: str,
    source_id: str,
    custom_property_names: frozenset,
    collision_seen: Optional[MutableSet[str]] = None,
) -> MutableMapping[str, Any]:
    """Return a copy of ``record`` with Insight metadata injected.

    HubSpot records are shaped as::

        {"id": "...", "createdAt": "...", "updatedAt": "...", "archived": bool,
         "properties": {...}, "associations": {...}}

    The envelope:
    - Flattens ``properties`` into top-level ``properties_{name}`` columns.
    - Splits custom properties (names in ``custom_property_names``) into the
      ``custom_fields`` JSON blob, keeping Bronze schema stable across portals.
    - Keeps ``associations`` as-is (already flattened to id-array form by the
      association helper before this call).
    - Adds ``tenant_id`` / ``source_id`` / ``unique_key`` / ``data_source`` /
      ``collected_at``.

    ``collision_seen`` gates one-shot-per-stream warnings on reserved-name
    collisions.
    """
    out: MutableMapping[str, Any] = {}
    customs: dict = {}
    properties = record.get("properties") or {}

    for key, value in record.items():
        if key == "properties":
            continue
        if key in _RESERVED_FIELD_NAMES:
            _warn_once(collision_seen, key)
            continue
        out[key] = value

    for prop_name, prop_value in properties.items():
        # HubSpot property names always land under a ``properties_`` prefix so
        # they can't collide with the unprefixed envelope reserved names; no
        # collision check needed here.
        if prop_name in custom_property_names:
            # Drop null/empty values — HubSpot returns every defined custom
            # property on every record, so a portal with hundreds of custom
            # fields (mostly empty per row) would otherwise bloat the JSON
            # blob 10–20× with dead keys.
            if prop_value is None or prop_value == "":
                continue
            customs[prop_name] = _truncate(prop_value)
        else:
            out[f"properties_{prop_name}"] = _truncate(prop_value)

    # ClickHouse stores JSON blobs as strings; serialize once.
    out["custom_fields"] = (
        json.dumps(customs, separators=(",", ":"), default=str) if customs else "{}"
    )

    hs_id = record.get("id")
    # Treat only None / empty string as missing — a numeric 0 is still a
    # legitimate id from HubSpot's perspective.
    if hs_id is None or hs_id == "":
        # Every HubSpot CRM object carries a numeric ``id``. An empty value
        # likely means a malformed response; derive a stable content hash so
        # Bronze ReplacingMergeTree doesn't collapse these rows into each
        # other on merge.
        logger.error(
            "HubSpot record missing id; unique_key derived from content hash (tenant=%s source=%s record_keys=%s)",
            tenant_id,
            source_id,
            list(record.keys())[:10],
        )
        canonical = json.dumps(record, sort_keys=True, default=str)
        hs_id = f"nohash:{hashlib.sha256(canonical.encode('utf-8')).hexdigest()[:16]}"

    out["tenant_id"] = tenant_id
    out["source_id"] = source_id
    out["unique_key"] = f"{tenant_id}-{source_id}-{hs_id}"
    out["data_source"] = DATA_SOURCE
    out["collected_at"] = _now_iso()
    return out


def _warn_once(seen: Optional[MutableSet[str]], key: str) -> None:
    if seen is None or key not in seen:
        logger.warning(
            "HubSpot field %r collides with Insight envelope field; original value dropped",
            key,
        )
        if seen is not None:
            seen.add(key)


ENVELOPE_FIELDS_SCHEMA = {
    "tenant_id": {"type": "string"},
    "source_id": {"type": "string"},
    "unique_key": {"type": "string"},
    "data_source": {"type": "string"},
    "collected_at": {"type": "string", "format": "date-time"},
    "custom_fields": {"type": "string"},
}


def inject_envelope_properties(schema: MutableMapping[str, Any]) -> MutableMapping[str, Any]:
    """Add envelope field definitions to a JSON schema.

    Used when advertising per-stream schemas so the destination creates
    columns for the envelope fields alongside the HubSpot fields.
    """
    props = schema.setdefault("properties", {})
    for name, spec in ENVELOPE_FIELDS_SCHEMA.items():
        props[name] = spec
    return schema
