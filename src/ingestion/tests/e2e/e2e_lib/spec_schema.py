"""JSON Schema for fixtures/<name>/spec.yaml.

A fixture's spec.yaml is the imperative skin over the declarative CSV folder:
it tells the runner which API endpoint to hit, what request body to send,
which dbt models to build, and how to compare the response against the
expected CSV. The schema is versioned (`spec_version: 1`) so the runner
can refuse fixtures from a future format without silently misinterpreting them.
"""

from __future__ import annotations

SPEC_SCHEMA: dict = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "title": "Bronze-to-API E2E fixture spec",
    "type": "object",
    "required": [
        "spec_version",
        "endpoint",
        "request_body",
        "key_columns",
    ],
    "additionalProperties": False,
    "properties": {
        "spec_version": {
            "type": "integer",
            "enum": [1],
            "description": "Format version. The runner refuses unknown majors.",
        },
        "description": {
            "type": "string",
            "description": "Human-readable note about what this fixture exercises.",
        },
        "endpoint": {
            "type": "string",
            "pattern": r"^/v\d+/.+",
            "description": "API path, e.g. `/v1/metrics/{metric_id}/query`. `{metric_id}` is interpolated from `metric_id` below.",
        },
        "method": {
            "type": "string",
            "enum": ["GET", "POST", "PUT", "DELETE"],
            "default": "POST",
        },
        "metric_id": {
            "type": "string",
            "pattern": r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$",
            "description": "Metric UUID. Resolved at runtime from MariaDB (auto-seeded by analytics-api migrations) OR provided via seed/metrics.yaml.",
        },
        "request_body": {
            "type": "object",
            "description": "Body to POST as JSON. May reference `{metric_id}` placeholder.",
        },
        "dbt_selector": {
            "type": "string",
            "minLength": 1,
            "description": "Passed verbatim to `dbt build --select`. e.g. `+silver_people+`.",
        },
        "key_columns": {
            "type": "array",
            "items": {"type": "string"},
            "minItems": 1,
            "description": "Columns used to sort rows for stable diff. MUST exist in expected/response.csv.",
        },
        "float_tolerance": {
            "type": "number",
            "exclusiveMinimum": 0,
            "default": 1e-6,
            "description": "Absolute tolerance for numeric column comparison.",
        },
    },
}
