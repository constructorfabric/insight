"""What `normalize_catalog_to_append.py` promises the connection PATCH.

The catalog it emits is built from the discover response, not from whatever the
connection already carries, and every advertised stream comes out selected.
That is what makes a stream added by a version bump reach an existing
installation: reconcile republishes the definition, re-discovers with the cache
disabled, and PATCHes this catalog in. Without it a new stream would sync only
where somebody refreshed the connection by hand.

Run: pytest src/ingestion/reconcile-connectors/tests
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

NORMALIZER = Path(__file__).resolve().parents[1] / "python" / "normalize_catalog_to_append.py"


def _stream(name: str, *, cursor: list[str] | None = None) -> dict:
    stream: dict = {
        "name": name,
        "jsonSchema": {"type": "object", "properties": {"unique_key": {"type": "string"}}},
        "supportedSyncModes": ["full_refresh", "incremental"] if cursor else ["full_refresh"],
    }
    if cursor:
        stream["defaultCursorField"] = cursor
    return stream


def _normalize(streams: list[dict]) -> dict:
    payload = json.dumps({"catalog": {"streams": [{"stream": s} for s in streams]}})
    done = subprocess.run(
        [sys.executable, str(NORMALIZER)],
        input=payload,
        capture_output=True,
        text=True,
        encoding="utf-8",
        check=True,
    )
    return json.loads(done.stdout)


def test_a_stream_the_connection_has_never_seen_comes_out_selected() -> None:
    """The upgrade path for a connector that gains a stream: the bump
    republishes the definition and this catalog is PATCHed onto the existing
    connection, so the new stream must arrive enabled with no operator step."""
    out = _normalize([_stream("repositories", cursor=["updated_on"]), _stream("brand_new")])

    selected = {e["stream"]["name"]: e["config"]["selected"] for e in out["streams"]}
    assert selected == {"repositories": True, "brand_new": True}


def test_every_stream_appends_and_keeps_all_its_fields() -> None:
    """Append-only at the destination, and no inherited field exclusion: an
    update PATCH must not carry a stale `selectedFields` list."""
    out = _normalize([_stream("brand_new")])

    config = out["streams"][0]["config"]
    assert config["destinationSyncMode"] == "append"
    assert config["fieldSelectionEnabled"] is False
    assert "selectedFields" not in config


def test_a_cursor_bearing_stream_is_incremental_and_a_bare_one_is_not() -> None:
    """The sync mode follows what the stream advertises, so a full-refresh
    probe stream does not acquire a cursor it cannot honour."""
    out = _normalize([_stream("with_cursor", cursor=["updated_on"]), _stream("without")])

    by_name = {e["stream"]["name"]: e["config"] for e in out["streams"]}
    assert by_name["with_cursor"]["syncMode"] == "incremental"
    assert by_name["with_cursor"]["cursorField"] == ["updated_on"]
    assert by_name["without"]["syncMode"] == "full_refresh"
    assert "cursorField" not in by_name["without"]
