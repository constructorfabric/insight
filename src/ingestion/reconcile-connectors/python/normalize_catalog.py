#!/usr/bin/env python3
# ---------------------------------------------------------------------------
# normalize_catalog.py
#
# Read an Airbyte discover_schema response from stdin and emit a syncCatalog
# JSON suitable for connections/create. Every stream is forced to
# destinationSyncMode=append_dedup with primaryKey=[["unique_key"]]: the
# destination then creates the bronze table as ReplacingMergeTree ORDER BY
# unique_key itself (engine-level dedup, same append-only insert path).
# A stream without a `unique_key` schema property fails the run — it would
# land as an ever-duplicating table. syncMode=incremental when a default
# cursor is advertised (default_cursor_field non-empty OR
# source_defined_cursor=true); otherwise full_refresh.
#
# @cpt-algo: cpt-insightspec-algo-reconcile-normalize-catalog-append-only:p1
# ---------------------------------------------------------------------------

import json
import sys

UNIQUE_KEY = "unique_key"


def _field(stream: dict, *keys):
    # The /sources/discover_schema API response serialises AirbyteStream in
    # camelCase (supportedSyncModes, sourceDefinedCursor, defaultCursorField);
    # the protocol/stored catalog uses snake_case. Read camelCase first, fall
    # back to snake_case so this is robust across Airbyte versions. Reading
    # only snake_case (the original bug) silently made EVERY stream
    # full_refresh because the camelCase keys never matched.
    for k in keys:
        v = stream.get(k)
        if v is not None:
            return v
    return None


def _stream_supports_incremental(stream: dict) -> bool:
    modes = _field(stream, "supportedSyncModes", "supported_sync_modes") or []
    if "incremental" not in modes:
        return False
    if _field(stream, "sourceDefinedCursor", "source_defined_cursor") is True:
        return True
    if _field(stream, "defaultCursorField", "default_cursor_field"):
        return True
    return False


def _stream_has_unique_key(stream: dict) -> bool:
    schema = _field(stream, "jsonSchema", "json_schema") or {}
    return UNIQUE_KEY in (schema.get("properties") or {})


def normalize(discover_response: dict) -> dict:
    catalog = discover_response.get("catalog") or {}
    raw_streams = catalog.get("streams") or []

    # unique_key is the dedup key of every bronze table (ReplacingMergeTree
    # ORDER BY unique_key, created by the destination). A stream without it
    # would land as an ever-duplicating table, so it is a hard error, not a
    # degraded mode.
    keyless = [
        str(_field(entry.get("stream") or entry, "name") or "<unnamed>")
        for entry in raw_streams
        if not _stream_has_unique_key(entry.get("stream") or entry)
    ]
    if keyless:
        raise ValueError(
            f"streams without a {UNIQUE_KEY} schema property: {', '.join(sorted(keyless))} —"
            " every stream must carry the identity stamp"
            f" (tenant_id, source_id, {UNIQUE_KEY})"
        )

    out_streams = []
    for entry in raw_streams:
        stream = entry.get("stream") or entry
        sync_mode = "incremental" if _stream_supports_incremental(stream) else "full_refresh"
        cfg = {
            "syncMode": sync_mode,
            "destinationSyncMode": "append_dedup",
            "primaryKey": [[UNIQUE_KEY]],
            # Per ADR-0015: every stream the source advertises is enabled.
            "selected": True,
            # Per ADR-0015: every field in jsonSchema is enabled. Explicit
            # `fieldSelectionEnabled: false` (no selectedFields list) means
            # Airbyte syncs all advertised properties; emitted explicitly so
            # an update PATCH does not inherit a stale exclusion list from a
            # prior connection state.
            "fieldSelectionEnabled": False,
        }
        cursor = _field(stream, "defaultCursorField", "default_cursor_field") or []
        if sync_mode == "incremental" and cursor:
            cfg["cursorField"] = cursor
        out_streams.append({"stream": stream, "config": cfg})
    return {"streams": out_streams}


def main() -> int:
    payload = json.load(sys.stdin)
    try:
        normalized = normalize(payload)
    except ValueError as e:
        print(f"normalize_catalog: {e}", file=sys.stderr)
        return 1
    json.dump(normalized, sys.stdout)
    return 0


if __name__ == "__main__":
    sys.exit(main())
