from __future__ import annotations

from collections import Counter
from collections.abc import Callable

import pytest
from insight_stand import ApiClient, Manifest, PersonaSession, analytics_path
from insight_stand.api import JsonValue

from ..schemas.analytics import MetricDrilldownResponse
from .team_numbers import (
    BUGS_FIXED,
    CLOSED,
    CLOSED_TASK,
    GOLDEN_FIELDS,
    Cell,
    _csv_matrix,
    _csv_rows,
    _golden,
    _kind,
    _period_values,
    _slices,
    _xlsx_matrix,
    _xlsx_rows,
)

DRILLDOWN = analytics_path("/v1/metric-drilldown")
DRILLDOWN_EXPORT = analytics_path("/v1/metric-drilldown/export")

PAGE_LIMIT = 25


def _drilldown_request(
    metric_key: str, person_id: str, start: str, end: str
) -> dict[str, JsonValue]:
    return {
        "metric_key": metric_key,
        "entity": {"type": "person", "id": person_id},
        "period": {"from": start, "to": end},
        "filters": [],
        "display_dimensions": [],
    }


def _walk(
    api: ApiClient, request: dict[str, JsonValue]
) -> tuple[MetricDrilldownResponse, list[list[Cell]]]:
    first: MetricDrilldownResponse | None = None
    rows: list[list[Cell]] = []
    seen: set[str] = set()
    cursor: str | None = None

    while True:
        page_request: dict[str, JsonValue] = {**request, "limit": PAGE_LIMIT}
        if cursor is not None:
            page_request["cursor"] = cursor

        response = api.post(DRILLDOWN, json_body=page_request)
        assert response.status_code == 200, f"{response.status_code} {response.text[:300]}"
        page = response.parse(MetricDrilldownResponse)
        assert page.not_ready is None, f"evidence is not ready: {page.not_ready}"
        assert len(page.rows) <= PAGE_LIMIT, f"a page of {len(page.rows)} rows past the limit"

        if first is None:
            first = page
        assert page.columns == first.columns, "the columns changed between pages"
        rows += [[row.values[column.key] for column in first.columns] for row in page.rows]

        cursor = page.next_cursor
        if cursor is None:
            return first, rows
        assert cursor not in seen, f"the cursor repeated: {cursor!r}"
        seen.add(cursor)


@pytest.mark.requires_seed("dev_lead", "development_ic")
@pytest.mark.reliability
def test_the_evidence_behind_a_team_members_closed_issues_is_the_seeds_own_list(
    session_for: Callable[[str], PersonaSession], stand_manifest: Manifest
) -> None:
    golden = _golden(stand_manifest)
    api = session_for("dev_lead").client
    member = stand_manifest.fixture("development_ic")
    expected = golden.per_person.get(member.email)
    assert expected is not None, f"golden_metrics carries no entry for {member.email}"

    refs: dict[str, Counter[str]] = {key: Counter() for key in GOLDEN_FIELDS}
    for start, end in _slices(golden):
        values = _period_values(api, [member.uuid], start, end)

        walked = {}
        for key in GOLDEN_FIELDS:
            page, rows = walked[key] = _walk(api, _drilldown_request(key, member.uuid, start, end))
            keys = [column.key for column in page.columns]
            assert "ref" in keys, f"{key} evidence carries no ref column: {keys}"
            assert len(rows) == (values[key, member.uuid] or 0), (
                f"{key} {start}..{end}: {len(rows)} evidence rows behind a value of "
                f"{values[key, member.uuid]}"
            )
            refs[key].update(str(row[keys.index("ref")]) for row in rows)

        page, rows = walked[CLOSED]
        kinds = [_kind(column.type.value) for column in page.columns]
        labels = [column.label for column in page.columns]
        export = _drilldown_request(CLOSED, member.uuid, start, end)

        csv_matrix = _csv_matrix(api.post(DRILLDOWN_EXPORT, json_body={**export, "format": "csv"}))
        xlsx_matrix = _xlsx_matrix(
            api.post(DRILLDOWN_EXPORT, json_body={**export, "format": "xlsx"}), len(kinds)
        )

        assert csv_matrix[0] == labels
        assert xlsx_matrix[0] == labels
        assert _csv_rows(csv_matrix[1:], kinds) == rows, f"{start}..{end}: the CSV is not the pages"
        assert _xlsx_rows(xlsx_matrix[1:], kinds) == rows, (
            f"{start}..{end}: the XLSX is not the pages"
        )

    for key, field in GOLDEN_FIELDS.items():
        assert refs[key].total() == getattr(expected, field), (
            f"{key}: {refs[key].total()} evidence rows for {member.email} over "
            f"{golden.window}; the seed wrote {getattr(expected, field)}"
        )
    assert refs[BUGS_FIXED] + refs[CLOSED_TASK] == refs[CLOSED], (
        "the closed bugs and closed tasks are not, issue for issue, the closed issues"
    )
