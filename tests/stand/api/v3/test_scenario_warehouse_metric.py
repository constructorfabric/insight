from __future__ import annotations

from collections.abc import Callable, Iterator
from datetime import date, timedelta
from typing import Final

import pytest
from insight_stand import (
    ADMIN_OPERATOR_FIXTURE,
    ApiClient,
    ApiResponse,
    GoldenTasks,
    JsonValue,
    Manifest,
    PersonaSession,
    analytics_path,
    scratch_name,
    v3_path,
)

from .. import scratch
from ..analytics import MAX_QUERY_SPAN_DAYS
from ..schemas import MetricResultsResponse, PeriodView
from ..schemas.v3 import Problem
from ..schemas.v3_untyped import (
    CreatedId,
    MetricDefinition,
    MetricEntry,
    Page,
    RunResult,
    TableDetail,
    TableList,
)

_GOLD_DATABASE: Final[str] = "insight"
_GOLD_TABLE: Final[str] = "task_metric_observations"
_MEASURE: Final[str] = "tasks_closed"
_BUILT_IN_METRIC: Final[str] = "tasks.closed"
_READ_COLUMNS: Final[frozenset[str]] = frozenset(
    {"entity_id", "measure_key", "metric_date", "value"}
)
_ADMIN_ONLY: Final[str] = "admin role required for this operation"


def _expect(response: ApiResponse, status: int) -> ApiResponse:
    assert response.status_code == status, (
        f"{response.url}: expected {status}, got {response.status_code} {response.text[:300]}"
    )
    return response


def _golden_tasks(manifest: Manifest) -> GoldenTasks:
    golden = manifest.golden_metrics
    if golden is None or golden.tasks is None:
        pytest.skip(
            f"the manifest at {manifest.source_path} carries no readable golden task totals"
        )
    if "silver" not in manifest.seeded:
        pytest.skip("the stand was seeded without the silver step the golden totals describe")
    return golden.tasks


def _window(tasks: GoldenTasks) -> tuple[date, date]:
    start, _, end = tasks.window.partition("..")
    return date.fromisoformat(start), date.fromisoformat(end)


def _closed_in_window_body(person_key: str, start: date, end: date) -> dict[str, JsonValue]:
    return {
        "database": _GOLD_DATABASE,
        "table": _GOLD_TABLE,
        "fields": [{"column": "value", "type": "float", "agg": "sum", "as_name": "closed"}],
        "filters": [
            {"column": "measure_key", "type": "string", "op": "eq", "value": _MEASURE},
            {"column": "entity_id", "type": "string", "op": "eq", "value": person_key},
            {"column": "metric_date", "type": "string", "op": "gte", "value": start.isoformat()},
            {"column": "metric_date", "type": "string", "op": "lte", "value": end.isoformat()},
        ],
    }


def _built_in_total(client: ApiClient, person_id: str, start: date, end: date) -> float:
    total = 0.0

    while start <= end:
        slice_end = min(start + timedelta(days=MAX_QUERY_SPAN_DAYS), end)
        response = client.post(
            analytics_path("/v1/metric-results"),
            json_body={
                "entity": {"type": "person", "ids": [person_id]},
                "period": {"from": start.isoformat(), "to": slice_end.isoformat()},
                "metrics": [{"metric_key": _BUILT_IN_METRIC, "views": [{"view": "period"}]}],
            },
        )

        for metric in _expect(response, 200).parse(MetricResultsResponse).metrics:
            for view in metric.root.views:
                assert isinstance(view.root, PeriodView)
                total += sum(value.value or 0 for value in view.root.values)

        start = slice_end + timedelta(days=1)

    return total


@pytest.fixture
def authored(v3_admin: ApiClient) -> Iterator[list[str]]:
    ids: list[str] = []
    yield ids

    for metric_id in ids:
        v3_admin.delete(v3_path(f"/v1/metrics/{metric_id}"))


@pytest.mark.reliability
@pytest.mark.requires_seed(ADMIN_OPERATOR_FIXTURE, "dev_lead", "development_ic")
def test_a_metric_over_the_gold_table_agrees_with_the_built_in_dashboard_and_the_seed(
    v3_admin: ApiClient,
    authored: list[str],
    session_for: Callable[[str], PersonaSession],
    stand_manifest: Manifest,
) -> None:
    tasks = _golden_tasks(stand_manifest)
    start, end = _window(tasks)
    person = stand_manifest.fixture("development_ic")
    expected = tasks.per_person[person.email].tasks_closed

    tables = _expect(
        v3_admin.get(v3_path("/v1/tables"), params={"database": _GOLD_DATABASE}), 200
    ).parse(TableList)
    assert (_GOLD_DATABASE, _GOLD_TABLE, "gold") in {
        (table.database, table.table, table.layer) for table in tables.tables
    }

    detail = _expect(
        v3_admin.get(v3_path(f"/v1/tables/{_GOLD_DATABASE}/{_GOLD_TABLE}")), 200
    ).parse(TableDetail)
    assert {column.name for column in detail.columns} >= _READ_COLUMNS

    name = scratch_name("closed")
    body = _closed_in_window_body(person.email, start, end)
    created = _expect(
        v3_admin.post(v3_path("/v1/metrics"), json_body={"name": name, "body": body}), 201
    ).parse(CreatedId)
    authored.append(created.id)
    scratch.track(v3_path("/v1/metrics"), "id", created.id, params={"q": name, "limit": 200})

    found = _expect(v3_admin.get(v3_path("/v1/metrics"), params={"q": name}), 200).parse(
        Page[MetricEntry]
    )
    assert [entry.id for entry in found.items] == [created.id]

    stored = _expect(v3_admin.get(v3_path(f"/v1/metrics/{created.id}")), 200).parse(
        MetricDefinition
    )
    assert (stored.name, stored.body) == (name, body)

    run = _expect(v3_admin.post(v3_path(f"/v1/metrics/{created.id}/run")), 200).parse(RunResult)
    built_in = _built_in_total(session_for("dev_lead").client, person.uuid, start, end)

    assert run.rows == [[expected]], (
        f"the v3 metric over {_GOLD_DATABASE}.{_GOLD_TABLE} counted {run.rows} closes for "
        f"{person.email} over {tasks.window}; the seed closed {expected}"
    )
    assert built_in == expected, (
        f"analytics {_BUILT_IN_METRIC} answered {built_in} for {person.email} over "
        f"{tasks.window}; the seed closed {expected}"
    )


@pytest.mark.security
@pytest.mark.requires_seed(ADMIN_OPERATOR_FIXTURE, "dev_lead", "development_ic")
def test_a_lead_may_neither_author_nor_list_metrics(
    v3_admin: ApiClient,
    session_for: Callable[[str], PersonaSession],
    stand_manifest: Manifest,
) -> None:
    lead = session_for("dev_lead").client
    name = scratch_name("lead-closed")
    person = stand_manifest.fixture("development_ic")
    body = _closed_in_window_body(person.email, date(2026, 1, 1), date(2026, 1, 31))

    for refused in (
        lead.post(v3_path("/v1/metrics"), json_body={"name": name, "body": body}),
        lead.get(v3_path("/v1/metrics"), params={"q": name}),
    ):
        assert _expect(refused, 403).parse(Problem).context["reason"] == _ADMIN_ONLY

    found = _expect(v3_admin.get(v3_path("/v1/metrics"), params={"q": name}), 200).parse(
        Page[MetricEntry]
    )
    assert found.items == []
