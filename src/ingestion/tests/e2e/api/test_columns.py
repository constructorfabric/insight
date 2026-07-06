"""Contract: GET /v1/columns · GET /v1/columns/{table} — column catalog reads.

`table_columns` has no seed migration, so the `seeded_columns` fixture inserts
two rows (two distinct tables) directly into MariaDB — the only write path
that exists — making both assertions run against data instead of vacuously
against an empty set.
"""

from __future__ import annotations

import pytest

pytestmark = pytest.mark.api


def test_list_columns_200(api, seeded_columns: dict) -> None:
    """The unfiltered list answers the {items: [...]} envelope and includes
    platform-visible (NULL-tenant) rows."""
    r = api.get("/v1/columns")
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    tables = {col["clickhouse_table"] for col in r.json()["items"]}
    assert seeded_columns["table_a"] in tables
    assert seeded_columns["table_b"] in tables


def test_table_columns_200(api, seeded_columns: dict) -> None:
    """The per-table read actually filters: it returns the requested table's
    rows and nothing from the other seeded table."""
    r = api.get(f"/v1/columns/{seeded_columns['table_a']}")
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    items = r.json()["items"]
    assert items, "seeded table must not filter down to an empty set"
    assert {col["clickhouse_table"] for col in items} == {seeded_columns["table_a"]}
    assert "metric_value" in {col["field_name"] for col in items}
