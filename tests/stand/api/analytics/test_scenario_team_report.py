from __future__ import annotations

from collections import defaultdict
from collections.abc import Callable, Sequence

import pytest
from insight_stand import ApiResponse, Manifest, Person, PersonaSession, analytics_path
from insight_stand.api import JsonValue

from ..schemas import PROBLEM_CONTENT_TYPE, ProblemDocument, ReportPreviewResponse
from . import query_window
from .team_numbers import (
    GOLDEN_FIELDS,
    _csv_matrix,
    _csv_rows,
    _golden,
    _kind,
    _period_values,
    _task_metric_labels,
    _team,
    _uuids,
    _window,
    _xlsx_matrix,
    _xlsx_rows,
)

REPORT_PREVIEW = analytics_path("/v1/reports/preview")
REPORT_EXPORT = analytics_path("/v1/reports/export")


def _refusal(response: ApiResponse) -> ProblemDocument:
    assert response.status_code == 403, f"{response.status_code}: {response.text[:300]}"
    assert response.content_type.startswith(PROBLEM_CONTENT_TYPE), response.content_type
    assert response.headers.get("content-disposition") is None, "a refusal offered a file"

    document = response.parse(ProblemDocument)
    assert document.status == 403
    return document


def _report_recipe(team: Sequence[Person], start: str, end: str) -> dict[str, JsonValue]:
    return {
        "subject": {"type": "people", "ids": list(_uuids(team))},
        "period": {"from": start, "to": end},
        "granularity": "quarter",
        "metric_keys": list(GOLDEN_FIELDS),
    }


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_a_leads_team_report_exports_the_previewed_rows_and_the_dashboards_numbers(
    session_for: Callable[[str], PersonaSession], stand_manifest: Manifest
) -> None:
    golden = _golden(stand_manifest)
    api = session_for("dev_lead").client
    team = _team(stand_manifest, "dev_lead")
    labels = _task_metric_labels(api)
    start, end = _window(golden)
    recipe = _report_recipe(team, start.isoformat(), end.isoformat())

    response = api.post(REPORT_PREVIEW, json_body=recipe)
    assert response.status_code == 200, f"preview: {response.status_code} {response.text[:300]}"
    preview = response.parse(ReportPreviewResponse)
    assert preview.not_ready is None, f"the report is not ready: {preview.not_ready}"

    keys = [column.key for column in preview.columns]
    kinds = [_kind(column.data_type.value) for column in preview.columns]
    header = [column.label for column in preview.columns]
    assert {key: header[keys.index(key)] for key in labels} == labels

    csv_matrix = _csv_matrix(api.post(REPORT_EXPORT, json_body={**recipe, "format": "csv"}))
    xlsx_matrix = _xlsx_matrix(
        api.post(REPORT_EXPORT, json_body={**recipe, "format": "xlsx"}), len(keys)
    )
    assert csv_matrix[0] == header
    assert xlsx_matrix[0] == header

    exported = _csv_rows(csv_matrix[1:], kinds)
    assert _xlsx_rows(xlsx_matrix[1:], kinds) == exported
    assert len(exported) == preview.total_rows

    previewed = [[None if cell is None else cell.root for cell in row.root] for row in preview.rows]
    assert previewed, "the preview carried no rows"
    assert previewed == exported[: len(previewed)]

    table = [dict(zip(keys, row, strict=True)) for row in exported]
    assert list(dict.fromkeys(row["email"] for row in table)) == [p.email for p in team]
    periods = list(dict.fromkeys((str(row["from"]), str(row["to"])) for row in table))
    assert len(table) == len(team) * len(periods)

    by_email = {person.email: person for person in team}
    totals: defaultdict[tuple[str, str], float] = defaultdict(float)
    for period_from, period_to in periods:
        values = _period_values(api, _uuids(team), period_from, period_to)
        for row in table:
            if (row["from"], row["to"]) != (period_from, period_to):
                continue
            person = by_email[str(row["email"])]
            for key in GOLDEN_FIELDS:
                assert row[key] == values[key, person.uuid], (
                    f"{person.email} {key} {period_from}..{period_to}: the report says "
                    f"{row[key]}, the dashboard {values[key, person.uuid]}"
                )
                totals[key, person.email] += float(row[key] or 0)

    for person in team:
        expected = golden.per_person.get(person.email)
        assert expected is not None, f"golden_metrics carries no entry for {person.email}"
        for key, field in GOLDEN_FIELDS.items():
            assert totals[key, person.email] == getattr(expected, field), (
                f"the report's {key} for {person.email} adds up to {totals[key, person.email]}; "
                f"the seed wrote {getattr(expected, field)}"
            )


@pytest.mark.requires_seed("dev_lead", "hr_lead")
@pytest.mark.security
def test_another_lead_is_refused_the_same_team_report_by_preview_and_both_exports(
    session_for: Callable[[str], PersonaSession], stand_manifest: Manifest
) -> None:
    start, end = query_window(stand_manifest)
    recipe = _report_recipe(_team(stand_manifest, "dev_lead"), start, end)

    owned = session_for("dev_lead").client.post(REPORT_PREVIEW, json_body=recipe)
    assert owned.status_code == 200, f"{owned.status_code} {owned.text[:300]}"
    owned.parse(ReportPreviewResponse)

    api = session_for("hr_lead").client
    previewed = _refusal(api.post(REPORT_PREVIEW, json_body=recipe))
    for file_format in ("csv", "xlsx"):
        exported = _refusal(api.post(REPORT_EXPORT, json_body={**recipe, "format": file_format}))
        assert (exported.type, exported.title, exported.detail, exported.context) == (
            previewed.type,
            previewed.title,
            previewed.detail,
            previewed.context,
        ), f"{file_format}: the export refuses differently from the preview"
