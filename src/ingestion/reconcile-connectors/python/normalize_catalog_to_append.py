#!/usr/bin/env python3
# ---------------------------------------------------------------------------
# normalize_catalog_to_append.py
#
# Read an Airbyte discover_schema response from stdin and emit a syncCatalog
# JSON suitable for connections/create. Every stream is forced to
# destinationSyncMode=append. syncMode=incremental when a default cursor is
# advertised (default_cursor_field non-empty OR source_defined_cursor=true);
# otherwise full_refresh.
#
# @cpt-algo: cpt-insightspec-algo-reconcile-normalize-catalog-append-only:p1
#
# Per cpt-dataflow-constraint-airbyte-append: append-only at destination
# avoids OOMs from append_dedup buffering and survives mid-stream pod kills.
# Dedup happens in silver via unique_key.
# ---------------------------------------------------------------------------

import json
import sys


def _stream_supports_incremental(stream: dict) -> bool:
    modes = stream.get("supported_sync_modes") or []
    if "incremental" not in modes:
        return False
    if stream.get("source_defined_cursor") is True:
        return True
    if stream.get("default_cursor_field"):
        return True
    return False


def normalize(discover_response: dict) -> dict:
    catalog = discover_response.get("catalog") or {}
    raw_streams = catalog.get("streams") or []
    out_streams = []
    for entry in raw_streams:
        stream = entry.get("stream") or entry
        sync_mode = "incremental" if _stream_supports_incremental(stream) else "full_refresh"
        cfg = {
            "syncMode": sync_mode,
            "destinationSyncMode": "append",
            "selected": True,
        }
        cursor = stream.get("default_cursor_field") or []
        if sync_mode == "incremental" and cursor:
            cfg["cursorField"] = cursor
        out_streams.append({"stream": stream, "config": cfg})
    return {"streams": out_streams}


def main() -> int:
    payload = json.load(sys.stdin)
    json.dump(normalize(payload), sys.stdout)
    return 0


if __name__ == "__main__":
    sys.exit(main())
