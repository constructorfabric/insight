"""`/v1/metric-drilldown` and its export — the evidence behind a metric value.

    POST /v1/metric-drilldown         200 · 400 empty-entity · 403 hidden person
    POST /v1/metric-drilldown/export  200 CSV/XLSX · 400 empty-entity

The 415 half is in `test_request_contracts.py`, swept over every body route.
"""

from __future__ import annotations

import csv
import io
import zipfile
from xml.etree import ElementTree

import pytest
from insight_stand import ApiClient, ApiResponse, Manifest, analytics_path
from insight_stand.api import JsonValue

from ..schemas import MetricResultsResponse, PeriodView, ProblemDocument
from ..schemas.analytics import MetricDrilldownResponse

DRILLDOWN = analytics_path("/v1/metric-drilldown")
DRILLDOWN_EXPORT = analytics_path("/v1/metric-drilldown/export")
METRIC_RESULTS = analytics_path("/v1/metric-results")
GIT_COMMITS = "git.commits"

#: Well-formed apart from the one field under test. An empty entity id is the
#: cheapest rejection that is unambiguously the HANDLER's — it needs no seeded
#: metric to reach, and no lookup can turn it into a 404 on the way.
_EMPTY_ENTITY_ID = ""


def _seeded_request(
    manifest: Manifest,
    *,
    entity_id: str | None = None,
    limit: int | None = 1,
    cursor: str | None = None,
) -> dict[str, JsonValue]:
    start, _, end = manifest.data_window.partition("..")
    request: dict[str, JsonValue] = {
        "metric_key": GIT_COMMITS,
        "entity": {
            "type": "person",
            "id": entity_id or manifest.fixture("dev_lead").uuid,
        },
        "period": {"from": start, "to": end},
        "filters": [{"dimension": "source", "values": ["github"]}],
        "display_dimensions": ["repository"],
    }
    if limit is not None:
        request["limit"] = limit
    if cursor is not None:
        request["cursor"] = cursor
    return request


def _all_rows(
    api: ApiClient, manifest: Manifest
) -> tuple[MetricDrilldownResponse, list[dict[str, object]]]:
    cursor: str | None = None
    cursors: set[str] = set()
    first: MetricDrilldownResponse | None = None
    rows: list[dict[str, object]] = []

    while True:
        response = api.post(
            DRILLDOWN,
            json_body=_seeded_request(manifest, cursor=cursor),
        )
        assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
        page = response.parse(MetricDrilldownResponse)
        if first is None:
            first = page
        else:
            assert page.columns == first.columns
            assert page.selection == first.selection
        rows.extend(row.values for row in page.rows)
        cursor = page.next_cursor
        if cursor is None:
            break
        assert cursor not in cursors, f"repeated pagination cursor {cursor!r}"
        cursors.add(cursor)

    assert first is not None
    return first, rows


def _period_value(api: ApiClient, manifest: Manifest) -> float:
    start, _, end = manifest.data_window.partition("..")
    person_id = manifest.fixture("dev_lead").uuid
    response = api.post(
        METRIC_RESULTS,
        json_body={
            "entity": {"type": "person", "ids": [person_id]},
            "period": {"from": start, "to": end},
            "metrics": [
                {
                    "metric_key": GIT_COMMITS,
                    "filters": [{"dimension": "source", "values": ["github"]}],
                    "views": [{"view": "period"}],
                }
            ],
        },
    )
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
    result = response.parse(MetricResultsResponse)
    assert len(result.metrics) == 1
    views = result.metrics[0].root.views
    assert len(views) == 1
    assert isinstance(views[0].root, PeriodView)
    values = views[0].root.values
    assert len(values) == 1
    assert values[0].entity_id == person_id
    assert values[0].value is not None
    return values[0].value


def _export(api: ApiClient, manifest: Manifest, file_format: str) -> ApiResponse:
    request = _seeded_request(manifest, limit=None)
    request["format"] = file_format
    return api.post(DRILLDOWN_EXPORT, json_body=request)


def _xlsx_rows(content: bytes) -> int:
    with zipfile.ZipFile(io.BytesIO(content)) as workbook:
        worksheet = ElementTree.fromstring(workbook.read("xl/worksheets/sheet1.xml"))
    return len(
        worksheet.findall(".//{http://schemas.openxmlformats.org/spreadsheetml/2006/main}row")
    )


@pytest.mark.requires_seed("dev_lead")
def test_git_commit_drilldown_pages_and_reconciles(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    first, rows = _all_rows(api, stand_manifest)
    person_id = stand_manifest.fixture("dev_lead").uuid

    assert first.selection.metric_key == GIT_COMMITS
    assert first.selection.entity.id == person_id
    assert first.selection.display_dimensions == ["repository"]
    assert [(item.dimension, item.values) for item in first.selection.filters] == [
        ("source", ["github"])
    ]
    column_keys = [column.key for column in first.columns]
    assert "repository" in column_keys
    assert "date" in column_keys
    assert rows
    assert all(set(row) == set(column_keys) for row in rows)
    assert len(rows) == _period_value(api, stand_manifest)


@pytest.mark.requires_seed("dev_lead")
def test_git_commit_drilldown_exports_all_rows(api: ApiClient, stand_manifest: Manifest) -> None:
    first, rows = _all_rows(api, stand_manifest)

    csv_response = _export(api, stand_manifest, "csv")
    assert csv_response.status_code == 200
    assert csv_response.content_type.startswith("text/csv")
    assert ".csv" in csv_response.headers.get("content-disposition", "")
    csv_rows = list(csv.reader(io.StringIO(csv_response.content.decode("utf-8-sig"))))
    assert csv_rows[0] == [column.label for column in first.columns]
    assert len(csv_rows) == len(rows) + 1

    xlsx_response = _export(api, stand_manifest, "xlsx")
    assert xlsx_response.status_code == 200
    assert xlsx_response.content_type == (
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    )
    assert ".xlsx" in xlsx_response.headers.get("content-disposition", "")
    assert _xlsx_rows(xlsx_response.content) == len(rows) + 1


@pytest.mark.requires_seed("dev_lead", "sales_ic")
def test_git_commit_drilldown_refuses_a_person_out_of_scope(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    outsider = stand_manifest.fixture("sales_ic")
    response = api.post(
        DRILLDOWN,
        json_body=_seeded_request(stand_manifest, entity_id=outsider.uuid),
    )
    assert response.status_code == 403, (
        f"asking about {outsider.email} answered {response.status_code}: {response.text[:300]}"
    )
    assert response.parse(ProblemDocument).status == 403


@pytest.mark.requires_seed("dev_lead")
def test_supported_metric_with_no_evidence_returns_an_empty_page(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    request = _seeded_request(stand_manifest)
    request["metric_key"] = "wiki.pages_created"
    request["filters"] = []
    request["display_dimensions"] = []

    response = api.post(DRILLDOWN, json_body=request)
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
    result = response.parse(MetricDrilldownResponse)
    assert result.selection.metric_key == "wiki.pages_created"
    assert result.rows == []
    assert result.next_cursor is None


def _request() -> dict[str, JsonValue]:
    return {
        "metric_key": "tasks.closed",
        "entity": {"type": "person", "id": _EMPTY_ENTITY_ID},
        "period": {"from": "2026-01-01", "to": "2026-01-31"},
        "filters": [],
        "display_dimensions": [],
        "limit": 100,
    }


def test_drilldown_400_empty_entity_id(api: ApiClient) -> None:
    response = api.post(DRILLDOWN, json_body=_request())
    assert response.status_code == 400, f"status={response.status_code} {response.text[:300]}"


def test_drilldown_export_400_empty_entity_id(api: ApiClient) -> None:
    """Same rejection, and it must happen before any format negotiation.

    `format` is what makes this operation different; validating the entity
    first is what keeps it the same route. A 200 with an empty CSV would be the
    regression worth catching here.
    """
    request = _request()
    request["format"] = "csv"
    del request["limit"]

    response = api.post(DRILLDOWN_EXPORT, json_body=request)
    assert response.status_code == 400, f"status={response.status_code} {response.text[:300]}"


@pytest.mark.parametrize(
    ("label", "entity_id"),
    [
        ("pre-cutover email", "somebody@example.com"),
        ("nil uuid", "00000000-0000-0000-0000-000000000000"),
    ],
)
@pytest.mark.parametrize("path", [DRILLDOWN, DRILLDOWN_EXPORT], ids=["drilldown", "export"])
def test_drilldown_400_for_a_key_that_is_not_a_person_id(
    api: ApiClient, path: str, label: str, entity_id: str
) -> None:
    """`entity.id` is a canonical person UUID since the identity cutover (#2098).

    Both spellings that used to work, or nearly, are now loud 400s — on the
    export route as well as the plain one, because a caller who has not
    migrated will hit whichever they were already using and a CSV of nothing is
    the least debuggable possible answer.
    """
    request = dict(_request())
    request["entity"] = {"type": "person", "id": entity_id}
    if path == DRILLDOWN_EXPORT:
        request["format"] = "csv"
        del request["limit"]

    response = api.post(path, json_body=request)
    assert response.status_code == 400, (
        f"{path} answered {response.status_code} to a {label}: {response.text[:300]}"
    )
