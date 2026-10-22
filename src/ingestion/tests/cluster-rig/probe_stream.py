#!/usr/bin/env python3
"""Write one append_dedup Bronze stream for destination-clickhouse.

Usage: probe_stream.py <dir> <rows>   writes <dir>/catalog.json and <dir>/input.jsonl
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

NAMESPACE = "bronze_cluster_rig"
STREAM = "probe"
EMITTED_AT_MS = 1760000000000


def catalog() -> dict[str, object]:
    schema = {
        "type": "object",
        "properties": {"unique_key": {"type": "string"}, "tenant_id": {"type": "string"}, "value": {"type": "integer"}},
    }

    return {
        "streams": [
            {
                "stream": {
                    "name": STREAM,
                    "namespace": NAMESPACE,
                    "json_schema": schema,
                    "supported_sync_modes": ["full_refresh"],
                },
                "sync_mode": "full_refresh",
                "destination_sync_mode": "append_dedup",
                "primary_key": [["unique_key"]],
                "generation_id": 1,
                "minimum_generation_id": 0,
                "sync_id": 1,
            }
        ]
    }


def messages(rows: int) -> list[dict[str, object]]:
    records = [
        {
            "type": "RECORD",
            "record": {
                "stream": STREAM,
                "namespace": NAMESPACE,
                "emitted_at": EMITTED_AT_MS,
                "data": {"unique_key": f"k{i}", "tenant_id": "rig", "value": i},
            },
        }
        for i in range(rows)
    ]
    complete = {
        "type": "TRACE",
        "trace": {
            "type": "STREAM_STATUS",
            "emitted_at": 1,
            "stream_status": {"stream_descriptor": {"name": STREAM, "namespace": NAMESPACE}, "status": "COMPLETE"},
        },
    }

    return [*records, complete]


def main(argv: list[str]) -> int:
    out, rows = Path(argv[0]), int(argv[1])

    (out / "catalog.json").write_text(json.dumps(catalog()))
    (out / "input.jsonl").write_text("".join(json.dumps(message) + "\n" for message in messages(rows)))

    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
