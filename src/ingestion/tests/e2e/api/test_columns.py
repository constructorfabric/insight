"""Contract: GET /v1/columns · GET /v1/columns/{table} — column catalog reads."""

from __future__ import annotations

import pytest

pytestmark = pytest.mark.api


def test_list_columns_200(api) -> None:
    """`table_columns` has no seed migration, so a fresh session may return an
    empty list — assert the {items: [...]} envelope, not row counts."""
    r = api.get("/v1/columns")
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    assert isinstance(r.json()["items"], list)


def test_table_columns_200(api) -> None:
    """Per-table filter answers the same envelope; every row echoes the table."""
    items = api.get("/v1/columns").json()["items"]
    table = items[0]["clickhouse_table"] if items else "gold_ic_kpis"
    r = api.get(f"/v1/columns/{table}")
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    per_table = r.json()["items"]
    assert isinstance(per_table, list)
    assert all(col["clickhouse_table"] == table for col in per_table)
