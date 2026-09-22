from __future__ import annotations

import json
import os
import subprocess
from datetime import UTC, datetime
from pathlib import Path
from uuid import uuid4

import clickhouse_connect
import pytest
from clickhouse_connect.driver.client import Client

DBT_DIR = Path(__file__).resolve().parents[2]


@pytest.fixture
def warehouse() -> Client:
    return clickhouse_connect.get_client(
        host=os.environ["CLICKHOUSE_HOST"],
        port=int(os.environ["CLICKHOUSE_PORT"]),
        username=os.environ["CLICKHOUSE_USER"],
        password=os.environ["CLICKHOUSE_PASSWORD"],
    )


def snapshot(warehouse: Client, tenant: str, source: str, entries: list[object], second: int) -> None:
    warehouse.insert(
        "bronze_bamboohr.whos_out",
        [
            [
                str(uuid4()),
                datetime(2025, 6, 1, 0, 0, second, tzinfo=UTC),
                tenant,
                source,
                json.dumps(entries),
                "2025-05-01",
                "2025-05-31",
            ]
        ],
        column_names=[
            "_airbyte_raw_id",
            "_airbyte_extracted_at",
            "tenant_id",
            "source_id",
            "entries_json",
            "window_start",
            "window_end",
        ],
    )


def build() -> None:
    subprocess.run(
        [
            os.environ["DBT_EXECUTABLE"],
            "build",
            "--profiles-dir",
            os.environ["DBT_PROFILES_DIR"],
            "--select",
            "bamboohr__person_absences",
            "class_person_absences",
        ],
        cwd=DBT_DIR,
        check=True,
        capture_output=True,
        text=True,
    )


def intervals(warehouse: Client, tenant: str, source: str) -> list[tuple[str, str, str]]:
    return warehouse.query(
        "SELECT account_id, toString(start_date), toString(end_date) "
        "FROM silver.class_person_absences FINAL "
        "WHERE insight_tenant_id = {tenant:String} "
        "AND account_source_id = {source:String} ORDER BY account_id",
        parameters={"tenant": tenant, "source": source},
    ).result_rows


def test_latest_snapshot_replaces_intervals_without_clearing_other_sources(warehouse: Client) -> None:
    tenant = str(uuid4())
    other_tenant = str(uuid4())
    source = str(uuid4())
    other_source = str(uuid4())
    leave = {"type": "timeOff", "employeeId": 7, "start": "2025-04-28", "end": "2025-05-02"}
    snapshot(warehouse, tenant, source, [leave], 1)
    snapshot(warehouse, tenant, other_source, [leave], 1)
    snapshot(warehouse, other_tenant, source, [leave], 1)
    build()
    expected = [("7", "2025-05-01", "2025-05-02")]
    assert intervals(warehouse, tenant, source) == expected

    snapshot(warehouse, tenant, source, [], 3)
    snapshot(warehouse, tenant, source, [leave], 2)
    build()
    assert intervals(warehouse, tenant, source) == []
    assert intervals(warehouse, tenant, other_source) == expected
    assert intervals(warehouse, other_tenant, source) == expected

    build()
    assert intervals(warehouse, tenant, other_source) == expected


def test_only_valid_person_intervals_within_snapshot_window_are_promoted(warehouse: Client) -> None:
    tenant = str(uuid4())
    source = str(uuid4())
    leave = {"type": "timeOff", "employeeId": "employee-example", "start": "2025-05-31", "end": "2025-06-02"}
    entries: list[object] = [
        leave,
        leave,
        {**leave, "type": "holiday"},
        {**leave, "employeeId": ""},
        {**leave, "employeeId": None},
        {**leave, "employeeId": True},
        {**leave, "employeeId": {"id": 7}},
        {**leave, "start": "not-a-date"},
        {**leave, "end": None},
        {**leave, "start": "2025-05-20", "end": "2025-05-19"},
        {**leave, "start": "2025-06-01"},
        None,
        42,
        "invalid",
    ]
    snapshot(warehouse, tenant, source, entries, 1)
    build()
    assert intervals(warehouse, tenant, source) == [("employee-example", "2025-05-31", "2025-05-31")]
