from __future__ import annotations

import os
from collections.abc import Callable, Iterator, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Final

import pytest
from insight_stand import (
    ADMIN_OPERATOR_FIXTURE,
    ApiClient,
    ApiResponse,
    JsonValue,
    PersonaSession,
    scratch_name,
    v3_path,
)
from insight_stand.stand import PUBLISHED_HOST, candidate_env_files, parse_env_file

from .. import scratch
from ..schemas.v3 import Problem
from ..schemas.v3_untyped import (
    CreatedId,
    DashboardDefinition,
    DashboardEntry,
    Dataset,
    DatasetDependents,
    DatasetPage,
    DatasetRecords,
    Dependents,
    DrawnWidget,
    Drilldown,
    Items,
    Named,
    Page,
    RunResult,
    WidgetDefinition,
    WidgetEntry,
)

_REPO_ROOT: Final[Path] = Path(__file__).resolve().parents[4]
_CORE_PORT_KEY: Final[str] = "INSIGHT_V3_CORE_PORT"
_INGEST_TOKEN_KEY: Final[str] = "INSIGHT_V3_INGEST_TOKEN"
_COMPOSE_FALLBACK_INGEST_TOKEN: Final[str] = "insight-v3-core-local-dev-ingest-token"
_INGEST_TOKEN_HEADER: Final[str] = "x-insight-token"
_INGEST_BODY_LIMIT_BYTES: Final[int] = 1_048_576
_ADMIN_ONLY: Final[str] = "admin role required for this operation"
_PAGE_ROWS: Final[int] = 2


@dataclass(frozen=True)
class Record:
    day: str
    author: str
    lines: int

    def raw(self) -> dict[str, JsonValue]:
        return {"day": self.day, "author": self.author, "lines": self.lines}

    def key(self) -> tuple[str, str, int]:
        return self.day[:10], self.author, self.lines


RECORDS: Final[tuple[Record, ...]] = (
    Record("2026-10-01T09:00:00Z", "ana", 4),
    Record("2026-10-02T09:00:00Z", "ben", 4),
    Record("2026-10-03T09:00:00Z", "ana", 4),
    Record("2026-10-04T09:00:00Z", "ben", 7),
    Record("2026-10-05T09:00:00Z", "ana", 1),
)

DECLARATION_FIELDS: Final[list[JsonValue]] = [
    {"name": "day", "path": "day", "type": "datetime", "default_clock": True},
    {"name": "author", "path": "author", "type": "string"},
    {"name": "lines", "path": "lines", "type": "int"},
]


@dataclass(frozen=True)
class Ingest:
    client: ApiClient
    token: str

    def post(self, body: JsonValue, token: str | None) -> ApiResponse:
        headers = None if token is None else {_INGEST_TOKEN_HEADER: token}
        return self.client.post("/v1/raw-data", json_body=body, headers=headers)

    def feed(self, dataset: str, records: Sequence[Record]) -> None:
        for record in records:
            _expect(self.post({"dataset": dataset, "raw_data": record.raw()}, self.token), 204)


@dataclass(frozen=True)
class Drilled:
    figure: Named
    rows: Named
    stat: Named
    dashboard: Named


def _expect(response: ApiResponse, status: int) -> ApiResponse:
    assert response.status_code == status, (
        f"{response.url}: expected {status}, got {response.status_code} {response.text[:300]}"
    )
    return response


def _declare(client: ApiClient, removals: list[str]) -> Dataset:
    name = scratch_name("fed")
    declaration: JsonValue = {"title": name, "fields": DECLARATION_FIELDS}

    response = _expect(client.put(v3_path(f"/v1/datasets/{name}"), json_body=declaration), 200)
    scratch.track(v3_path("/v1/datasets"), "name", name, params={"q": name, "limit": 200})
    removals.append(v3_path(f"/v1/datasets/{name}"))

    return response.parse(Dataset)


def _create(
    client: ApiClient, removals: list[str], segment: str, body: dict[str, JsonValue]
) -> Named:
    name = scratch_name(segment)

    response = _expect(
        client.post(v3_path(f"/v1/{segment}"), json_body={"name": name, "body": body}), 201
    )
    created = response.parse(CreatedId).id
    scratch.track(v3_path(f"/v1/{segment}"), "id", created, params={"q": name, "limit": 200})
    removals.append(v3_path(f"/v1/{segment}/{created}"))

    return Named(id=created, name=name)


def _run(client: ApiClient, path: str) -> RunResult:
    return _expect(client.post(path), 200).parse(RunResult)


def _drawn(client: ApiClient, dashboard: str) -> list[DrawnWidget]:
    response = _expect(client.get(v3_path(f"/v1/dashboards/{dashboard}/widgets")), 200)
    return response.parse(Items[DrawnWidget]).items


def _holders(client: ApiClient, path: str) -> set[tuple[str, str]]:
    found = _expect(client.get(path), 200).parse(Dependents)
    assert found.hidden == 0
    return {(holder.kind, holder.id) for holder in found.holders}


def _walk(client: ApiClient, path: str) -> list[Drilldown]:
    pages: list[Drilldown] = []
    cursor: str | None = None

    while True:
        body: dict[str, JsonValue] = {
            "sort": {"key": "lines", "direction": "asc"},
            "limit": _PAGE_ROWS,
        }
        if cursor is not None:
            body["cursor"] = cursor

        page = _expect(client.post(path, json_body=body), 200).parse(Drilldown)
        pages.append(page)

        cursor = page.next_cursor
        if cursor is None:
            return pages
        assert len(pages) <= len(RECORDS), f"{path} kept handing out cursors past every record"


def _row_key(values: dict[str, JsonValue]) -> tuple[str, JsonValue, JsonValue]:
    return str(values["day"])[:10], values["author"], values["lines"]


@pytest.fixture(scope="module")
def ingest() -> Ingest:
    env_file = next(
        (path for path in candidate_env_files(_REPO_ROOT, os.environ) if path.is_file()), None
    )
    settings = parse_env_file(env_file) if env_file is not None else {}

    port = settings.get(_CORE_PORT_KEY, "").strip()
    if not port:
        pytest.skip(
            f"raw-data is taken on insight-v3-core's own port, and no stand env file sets "
            f"{_CORE_PORT_KEY}"
        )

    token = (os.environ.get(_INGEST_TOKEN_KEY) or settings.get(_INGEST_TOKEN_KEY) or "").strip()
    client = ApiClient(base_url=f"http://{PUBLISHED_HOST}:{port}", edge_fronted=False)
    return Ingest(client, token or _COMPOSE_FALLBACK_INGEST_TOKEN)


@pytest.fixture
def removals(v3_admin: ApiClient) -> Iterator[list[str]]:
    paths: list[str] = []
    yield paths

    for path in reversed(paths):
        v3_admin.delete(path)


@pytest.fixture
def fed_dataset(v3_admin: ApiClient, ingest: Ingest, removals: list[str]) -> Dataset:
    dataset = _declare(v3_admin, removals)
    ingest.feed(dataset.name, RECORDS)
    return dataset


@pytest.fixture
def drilled(v3_admin: ApiClient, fed_dataset: Dataset, removals: list[str]) -> Drilled:
    figure = _create(
        v3_admin,
        removals,
        "metrics",
        {
            "dataset": fed_dataset.name,
            "fields": [{"agg": "count", "type": "int", "as_name": "records"}],
        },
    )
    rows = _create(
        v3_admin,
        removals,
        "metrics",
        {
            "dataset": fed_dataset.name,
            "fields": [
                {"field": "day", "type": "string", "as_name": "day"},
                {"field": "author", "type": "string", "as_name": "author"},
                {"field": "lines", "type": "int", "as_name": "lines"},
            ],
        },
    )
    stat = _create(
        v3_admin,
        removals,
        "widgets",
        {"type": "stat", "metric": figure.id, "value": "records", "detail": rows.id},
    )
    dashboard = _create(
        v3_admin,
        removals,
        "dashboards",
        {"title": "Records", "items": [{"widget": stat.id}]},
    )

    return Drilled(figure=figure, rows=rows, stat=stat, dashboard=dashboard)


@pytest.mark.reliability
@pytest.mark.requires_seed(ADMIN_OPERATOR_FIXTURE)
def test_records_pushed_into_a_dataset_feed_a_metric_until_the_dataset_is_taken_away(
    v3_admin: ApiClient, ingest: Ingest, removals: list[str]
) -> None:
    dataset = _declare(v3_admin, removals)
    dataset_path = v3_path(f"/v1/datasets/{dataset.name}")
    records_path = f"{dataset_path}/records"

    listed = _expect(v3_admin.get(v3_path("/v1/datasets"), params={"q": dataset.name}), 200).parse(
        DatasetPage
    )
    assert listed.names == [dataset.name]
    assert (listed.items[0].title, listed.items[0].fields) == (
        dataset.name,
        len(DECLARATION_FIELDS),
    )

    opened = _expect(v3_admin.get(dataset_path), 200).parse(Dataset)
    assert opened == dataset

    ingest.feed(dataset.name, RECORDS)

    by_day = sorted(RECORDS, key=lambda record: record.day)
    for direction, expected in (("asc", by_day), ("desc", by_day[::-1])):
        page = _expect(
            v3_admin.get(records_path, params={"order_by": "day", "direction": direction}), 200
        ).parse(DatasetRecords)
        assert page.total == len(RECORDS)
        assert [record.raw_data for record in page.records] == [record.raw() for record in expected]

    metric = _create(
        v3_admin,
        removals,
        "metrics",
        {
            "dataset": dataset.name,
            "fields": [
                {"field": "lines", "type": "int", "agg": "sum", "as_name": "lines"},
                {"agg": "count", "type": "int", "as_name": "records"},
            ],
        },
    )
    metric_path = v3_path(f"/v1/metrics/{metric.id}")

    run = _run(v3_admin, f"{metric_path}/run")
    assert run.columns == ["lines", "records"]
    assert run.rows == [[sum(record.lines for record in RECORDS), len(RECORDS)]]

    dependents = _expect(v3_admin.get(f"{dataset_path}/dependents"), 200).parse(DatasetDependents)
    assert dependents.metrics == [metric.id]

    in_use = _expect(v3_admin.delete(dataset_path), 409).parse(Problem)
    assert [violation["type"] for violation in in_use.context["violations"]] == ["in_use"]

    _expect(v3_admin.delete(metric_path), 204)
    _expect(v3_admin.delete(dataset_path), 204)

    _expect(v3_admin.get(records_path), 404).parse(Problem)
    _expect(v3_admin.get(dataset_path), 404).parse(Problem)
    _expect(ingest.post({"dataset": dataset.name, "raw_data": RECORDS[0].raw()}, ingest.token), 404)


@pytest.mark.security
@pytest.mark.requires_seed(ADMIN_OPERATOR_FIXTURE)
def test_a_record_without_the_instance_token_is_refused_and_never_stored(
    v3_admin: ApiClient, ingest: Ingest, scratch_v3_dataset: Dataset
) -> None:
    body: JsonValue = {
        "dataset": scratch_v3_dataset.name,
        "raw_data": {"day": RECORDS[0].day, "lines": RECORDS[0].lines},
    }

    for token in (None, f"{ingest.token}-not-this-one"):
        refused = _expect(ingest.post(body, token), 401).parse(Problem)
        assert refused.context["reason"] == "INVALID_INGEST_TOKEN"

    page = _expect(
        v3_admin.get(v3_path(f"/v1/datasets/{scratch_v3_dataset.name}/records")), 200
    ).parse(DatasetRecords)
    assert page.total == 0


@pytest.mark.reliability
@pytest.mark.requires_seed(ADMIN_OPERATOR_FIXTURE)
def test_ingest_refuses_an_undeclared_dataset_and_an_oversized_record(
    v3_admin: ApiClient, ingest: Ingest, scratch_v3_dataset: Dataset
) -> None:
    undeclared = scratch_name("undeclared")
    record: dict[str, JsonValue] = {"day": RECORDS[0].day, "lines": RECORDS[0].lines}

    _expect(ingest.post({"dataset": undeclared, "raw_data": record}, ingest.token), 404).parse(
        Problem
    )
    catalogue = _expect(v3_admin.get(v3_path("/v1/datasets"), params={"q": undeclared}), 200).parse(
        DatasetPage
    )
    assert catalogue.names == []

    oversized = {**record, "padding": "x" * _INGEST_BODY_LIMIT_BYTES}
    too_large = _expect(
        ingest.post({"dataset": scratch_v3_dataset.name, "raw_data": oversized}, ingest.token),
        413,
    ).parse(Problem)
    assert [violation["reason"] for violation in too_large.context["field_violations"]] == [
        "TOO_LARGE"
    ]

    page = _expect(
        v3_admin.get(v3_path(f"/v1/datasets/{scratch_v3_dataset.name}/records")), 200
    ).parse(DatasetRecords)
    assert page.total == 0


@pytest.mark.reliability
@pytest.mark.requires_seed(ADMIN_OPERATOR_FIXTURE)
def test_a_dashboard_is_built_changed_and_dismantled_in_dependency_order(
    v3_admin: ApiClient, fed_dataset: Dataset, removals: list[str]
) -> None:
    lines_by_author: dict[str, int] = {}
    for record in sorted(RECORDS, key=lambda record: record.author):
        lines_by_author[record.author] = lines_by_author.get(record.author, 0) + record.lines

    metric_body: dict[str, JsonValue] = {
        "dataset": fed_dataset.name,
        "fields": [
            {"field": "author", "type": "string", "as_name": "author"},
            {"field": "lines", "type": "int", "agg": "sum", "as_name": "lines"},
        ],
        "group_by": ["author"],
        "order_by": {"field": "author"},
    }
    metric = _create(v3_admin, removals, "metrics", metric_body)
    metric_path = v3_path(f"/v1/metrics/{metric.id}")

    table_body: dict[str, JsonValue] = {
        "type": "table",
        "metric": metric.id,
        "columns": ["author", "lines"],
    }
    table = _create(v3_admin, removals, "widgets", table_body)
    bars = _create(
        v3_admin,
        removals,
        "widgets",
        {"type": "bar", "metric": metric.id, "x": "author", "y": "lines"},
    )
    table_path = v3_path(f"/v1/widgets/{table.id}")

    library = _expect(v3_admin.get(v3_path("/v1/widgets"), params={"q": metric.id}), 200).parse(
        Page[WidgetEntry]
    )
    assert {entry.id for entry in library.items} == {table.id, bars.id}
    assert {entry.metric_name for entry in library.items} == {metric.name}
    assert _expect(v3_admin.get(table_path), 200).parse(WidgetDefinition).body == table_body

    dashboard = _create(
        v3_admin,
        removals,
        "dashboards",
        {"title": "Lines by author", "items": [{"widget": table.id}, {"widget": bars.id}]},
    )
    dashboard_path = v3_path(f"/v1/dashboards/{dashboard.id}")
    table_run = f"{dashboard_path}/widgets/{table.id}/run"

    listed = _expect(
        v3_admin.get(v3_path("/v1/dashboards"), params={"q": dashboard.name}), 200
    ).parse(Page[DashboardEntry])
    assert [(entry.id, entry.access) for entry in listed.items] == [(dashboard.id, "owner")]

    opened = _expect(v3_admin.get(dashboard_path), 200).parse(DashboardDefinition)
    assert opened.body["items"] == [{"widget": table.id}, {"widget": bars.id}]
    assert [drawn.id for drawn in _drawn(v3_admin, dashboard.id)] == [table.id, bars.id]

    assert _run(v3_admin, table_run).rows == [
        [author, lines] for author, lines in lines_by_author.items()
    ]

    reordered: JsonValue = {
        "name": dashboard.name,
        "body": {"title": "Lines by author", "items": [{"widget": bars.id}, {"widget": table.id}]},
    }
    _expect(v3_admin.put(dashboard_path, json_body=reordered), 204)
    assert [drawn.id for drawn in _drawn(v3_admin, dashboard.id)] == [bars.id, table.id]

    narrower = {**table_body, "columns": ["lines"]}
    _expect(v3_admin.put(table_path, json_body={"name": table.name, "body": narrower}), 204)
    assert _drawn(v3_admin, dashboard.id)[1].body == narrower

    filtered = {
        **metric_body,
        "filters": [{"field": "author", "type": "string", "op": "eq", "value": "ben"}],
    }
    _expect(v3_admin.put(metric_path, json_body={"name": metric.name, "body": filtered}), 204)
    assert _run(v3_admin, table_run).rows == [["ben", lines_by_author["ben"]]]

    assert _holders(v3_admin, f"{metric_path}/dependents") == {
        ("widgets", table.id),
        ("widgets", bars.id),
    }
    assert _holders(v3_admin, f"{table_path}/dependents") == {("dashboards", dashboard.id)}
    assert _holders(v3_admin, f"{dashboard_path}/dependents") == set()

    for named in (metric_path, table_path):
        in_use = _expect(v3_admin.delete(named), 409).parse(Problem)
        assert [violation["type"] for violation in in_use.context["violations"]] == ["in_use"]

    for removed in (dashboard_path, table_path, v3_path(f"/v1/widgets/{bars.id}"), metric_path):
        _expect(v3_admin.delete(removed), 204)
        _expect(v3_admin.get(removed), 404)


@pytest.mark.reliability
@pytest.mark.requires_seed(ADMIN_OPERATOR_FIXTURE)
def test_drilling_into_a_widget_pages_every_row_behind_its_figure_exactly_once(
    v3_admin: ApiClient, drilled: Drilled
) -> None:
    figure = _run(v3_admin, v3_path(f"/v1/metrics/{drilled.figure.id}/run"))
    assert figure.rows == [[len(RECORDS)]]

    by_metric = _walk(v3_admin, v3_path(f"/v1/metrics/{drilled.rows.id}/drilldown"))
    by_widget = _walk(
        v3_admin,
        v3_path(f"/v1/dashboards/{drilled.dashboard.id}/widgets/{drilled.stat.id}/drilldown"),
    )

    assert [(page.selection, page.rows) for page in by_widget] == [
        (page.selection, page.rows) for page in by_metric
    ]
    assert {page.selection.metric for page in by_widget} == {drilled.rows.id}

    rows = [row.values for page in by_metric for row in page.rows]
    assert all(len(page.rows) <= _PAGE_ROWS for page in by_metric)
    assert len(rows) == figure.rows[0][0]
    assert [row["lines"] for row in rows] == sorted(record.lines for record in RECORDS)
    assert sorted(_row_key(row) for row in rows) == sorted(record.key() for record in RECORDS)


@pytest.mark.security
@pytest.mark.requires_seed(ADMIN_OPERATOR_FIXTURE, "dev_lead")
def test_a_dashboard_viewer_sees_the_figure_but_may_not_drill_into_the_rows_behind_it(
    v3_admin: ApiClient, drilled: Drilled, session_for: Callable[[str], PersonaSession]
) -> None:
    viewer = session_for("dev_lead")
    dashboard_path = v3_path(f"/v1/dashboards/{drilled.dashboard.id}")
    grant = f"{dashboard_path}/grants/{viewer.person.uuid}"

    _expect(v3_admin.put(grant, json_body={"level": "viewer"}), 204)
    try:
        figure = _run(viewer.client, f"{dashboard_path}/widgets/{drilled.stat.id}/run")
        assert figure.rows == [[len(RECORDS)]]

        for drilldown in (
            f"{dashboard_path}/widgets/{drilled.stat.id}/drilldown",
            v3_path(f"/v1/metrics/{drilled.rows.id}/drilldown"),
        ):
            refused = _expect(
                viewer.client.post(drilldown, json_body={"limit": _PAGE_ROWS}), 403
            ).parse(Problem)
            assert refused.context["reason"] == _ADMIN_ONLY
    finally:
        v3_admin.delete(grant)
