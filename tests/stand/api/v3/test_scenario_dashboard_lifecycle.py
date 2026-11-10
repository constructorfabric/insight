from __future__ import annotations

import pytest
from insight_stand import ADMIN_OPERATOR_FIXTURE, ApiClient, JsonValue, v3_path

from ..schemas.v3 import Problem
from ..schemas.v3_untyped import (
    DashboardDefinition,
    DashboardEntry,
    Dataset,
    Dependents,
    DrawnWidget,
    Items,
    Page,
    WidgetDefinition,
    WidgetEntry,
)
from .fed_records import RECORDS, _create, _expect, _run
from .fed_records import fed_dataset as fed_dataset
from .fed_records import ingest as ingest
from .fed_records import removals as removals


def _drawn(client: ApiClient, dashboard: str) -> list[DrawnWidget]:
    response = _expect(client.get(v3_path(f"/v1/dashboards/{dashboard}/widgets")), 200)
    return response.parse(Items[DrawnWidget]).items


def _holders(client: ApiClient, path: str) -> set[tuple[str, str]]:
    found = _expect(client.get(path), 200).parse(Dependents)
    assert found.hidden == 0
    return {(holder.kind, holder.id) for holder in found.holders}


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
