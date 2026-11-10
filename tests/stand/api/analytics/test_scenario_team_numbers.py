from __future__ import annotations

import csv
import datetime as dt
import io
import zipfile
from collections import Counter, defaultdict
from collections.abc import Callable, Sequence
from xml.etree import ElementTree

import pytest
from insight_stand import (
    ApiClient,
    ApiResponse,
    GoldenTasks,
    Manifest,
    Person,
    PersonaSession,
    analytics_path,
    identity_path,
)
from insight_stand.api import JsonValue

from ..identity.views import forest_emails
from ..schemas import (
    PROBLEM_CONTENT_TYPE,
    MetricDefinitionListResponse,
    MetricResultsResponse,
    PeriodView,
    ProblemDocument,
    ReportPreviewResponse,
    SubchartForest,
)
from ..schemas.analytics import MetricDrilldownResponse
from . import MAX_QUERY_SPAN_DAYS, query_window

METRIC_DEFINITIONS = analytics_path("/v1/metric-definitions")
METRIC_RESULTS = analytics_path("/v1/metric-results")
DRILLDOWN = analytics_path("/v1/metric-drilldown")
DRILLDOWN_EXPORT = analytics_path("/v1/metric-drilldown/export")
REPORT_PREVIEW = analytics_path("/v1/reports/preview")
REPORT_EXPORT = analytics_path("/v1/reports/export")
SUBCHART = identity_path("/v1/subchart")

CLOSED = "tasks.closed"
BUGS_FIXED = "tasks.bugs_fixed"
CLOSED_TASK = "tasks.closed_task"
GOLDEN_FIELDS = {CLOSED: "tasks_closed", BUGS_FIXED: "bugs_fixed", CLOSED_TASK: "closed_task"}

PAGE_LIMIT = 25
XLSX_CONTENT_TYPE = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
XLSX_MAIN = "{http://schemas.openxmlformats.org/spreadsheetml/2006/main}"
XLSX_EPOCH = dt.date(1899, 12, 30)

type Cell = str | float | None
type PeriodValues = dict[tuple[str, str], float | None]


def _golden(manifest: Manifest) -> GoldenTasks:
    golden = manifest.golden_metrics
    if golden is None or golden.tasks is None:
        pytest.skip(f"the manifest at {manifest.source_path} carries no golden task totals")
    if "silver" not in manifest.seeded:
        pytest.skip("the stand was seeded without the silver step the golden totals describe")
    return golden.tasks


def _window(tasks: GoldenTasks) -> tuple[dt.date, dt.date]:
    start, _, end = tasks.window.partition("..")
    return dt.date.fromisoformat(start), dt.date.fromisoformat(end)


def _slices(tasks: GoldenTasks) -> list[tuple[str, str]]:
    start, end = _window(tasks)

    slices: list[tuple[str, str]] = []
    while start <= end:
        slice_end = min(start + dt.timedelta(days=MAX_QUERY_SPAN_DAYS), end)
        slices.append((start.isoformat(), slice_end.isoformat()))
        start = slice_end + dt.timedelta(days=1)
    return slices


def _team(manifest: Manifest, lead: str) -> list[Person]:
    head = manifest.fixture(lead)
    members = [
        person
        for person in manifest.personas
        if person.team == head.team and person.uuid != head.uuid
    ]
    assert members, f"the roster gives {lead} no team members under team {head.team!r}"
    return [head, *members]


def _uuids(people: Sequence[Person]) -> list[str]:
    return [person.uuid for person in people]


def _assert_subchart_holds(session: PersonaSession, team: Sequence[Person]) -> None:
    response = session.client.get(SUBCHART)
    assert response.status_code == 200, f"subchart: {response.status_code} {response.text[:300]}"

    missing = {person.email for person in team} - forest_emails(response.parse(SubchartForest))
    assert not missing, f"{session.name}'s subchart does not hold their team: {sorted(missing)}"


def _task_metric_labels(api: ApiClient) -> dict[str, str]:
    response = api.get(METRIC_DEFINITIONS)
    assert response.status_code == 200, f"definitions: {response.status_code}"

    offered = {
        metric.metric_key: metric for metric in response.parse(MetricDefinitionListResponse).metrics
    }
    for key in GOLDEN_FIELDS:
        assert key in offered, f"the catalogue no longer offers {key}"
        assert offered[key].is_enabled, f"the catalogue offers {key} disabled"
    return {key: offered[key].label for key in GOLDEN_FIELDS}


def _ask_results(api: ApiClient, ids: Sequence[str], start: str, end: str) -> ApiResponse:
    return api.post(
        METRIC_RESULTS,
        json_body={
            "entity": {"type": "person", "ids": list(ids)},
            "period": {"from": start, "to": end},
            "metrics": [
                {"metric_key": key, "views": [{"view": "period"}]} for key in GOLDEN_FIELDS
            ],
        },
    )


def _period_values(api: ApiClient, ids: Sequence[str], start: str, end: str) -> PeriodValues:
    response = _ask_results(api, ids, start, end)
    assert response.status_code == 200, (
        f"{start}..{end}: {response.status_code} {response.text[:300]}"
    )

    values: PeriodValues = {}
    for metric in response.parse(MetricResultsResponse).metrics:
        for view in metric.root.views:
            assert isinstance(view.root, PeriodView), (
                f"asked for the period view and got {type(view.root).__name__}"
            )
            for value in view.root.values:
                values[metric.root.metric_key, value.entity_id] = value.value

    assert set(values) == {(key, uuid) for key in GOLDEN_FIELDS for uuid in ids}, (
        f"{start}..{end}: answered for {sorted(values)}"
    )
    return values


def _refusal(response: ApiResponse) -> ProblemDocument:
    assert response.status_code == 403, f"{response.status_code}: {response.text[:300]}"
    assert response.content_type.startswith(PROBLEM_CONTENT_TYPE), response.content_type
    assert response.headers.get("content-disposition") is None, "a refusal offered a file"

    document = response.parse(ProblemDocument)
    assert document.status == 403
    return document


def _kind(column_type: str) -> str:
    return column_type if column_type in ("date", "number") else "text"


def _csv_matrix(response: ApiResponse) -> list[list[str]]:
    assert response.status_code == 200, f"csv: {response.status_code} {response.text[:300]}"
    assert response.content_type.startswith("text/csv"), response.content_type
    assert ".csv" in response.headers.get("content-disposition", "")

    return list(csv.reader(io.StringIO(response.content.decode("utf-8-sig"))))


def _xlsx_column(reference: str) -> int:
    index = 0
    for character in reference:
        if character.isdigit():
            break
        index = index * 26 + ord(character) - ord("A") + 1
    return index - 1


def _xlsx_matrix(response: ApiResponse, width: int) -> list[list[Cell]]:
    assert response.status_code == 200, f"xlsx: {response.status_code} {response.text[:300]}"
    assert response.content_type == XLSX_CONTENT_TYPE, response.content_type
    assert ".xlsx" in response.headers.get("content-disposition", "")

    with zipfile.ZipFile(io.BytesIO(response.content)) as workbook:
        shared: list[str] = []
        if "xl/sharedStrings.xml" in workbook.namelist():
            strings = ElementTree.fromstring(workbook.read("xl/sharedStrings.xml"))
            shared = [
                "".join(node.text or "" for node in item.iter(f"{XLSX_MAIN}t"))
                for item in strings.findall(f"{XLSX_MAIN}si")
            ]
        sheet = ElementTree.fromstring(workbook.read("xl/worksheets/sheet1.xml"))

    matrix: list[list[Cell]] = []
    for row in sheet.iter(f"{XLSX_MAIN}row"):
        cells: list[Cell] = [None] * width
        for cell in row.findall(f"{XLSX_MAIN}c"):
            index = _xlsx_column(cell.get("r", ""))
            assert 0 <= index < width, f"cell {cell.get('r')} lies outside {width} columns"
            assert cell.find(f"{XLSX_MAIN}f") is None, f"cell {cell.get('r')} is a formula"

            kind = cell.get("t", "n")
            value = cell.find(f"{XLSX_MAIN}v")
            if kind == "inlineStr":
                cells[index] = "".join(node.text or "" for node in cell.iter(f"{XLSX_MAIN}t"))
            elif kind == "s":
                assert value is not None and value.text is not None
                cells[index] = shared[int(value.text)]
            elif value is not None and value.text is not None:
                cells[index] = float(value.text)
        matrix.append(cells)
    return matrix


def _from_csv(text: str, kind: str) -> Cell:
    if text == "":
        return None
    if kind == "number":
        return float(text)
    return text


def _from_xlsx(cell: Cell, kind: str) -> Cell:
    if kind == "date" and isinstance(cell, float):
        return (XLSX_EPOCH + dt.timedelta(days=cell)).isoformat()
    if cell == "":
        return None
    return cell


def _csv_rows(matrix: Sequence[Sequence[str]], kinds: Sequence[str]) -> list[list[Cell]]:
    for row in matrix:
        assert len(row) == len(kinds), f"csv: a row of {len(row)} cells for {len(kinds)} columns"
    return [
        [_from_csv(text, kind) for text, kind in zip(row, kinds, strict=True)] for row in matrix
    ]


def _xlsx_rows(matrix: Sequence[Sequence[Cell]], kinds: Sequence[str]) -> list[list[Cell]]:
    return [
        [_from_xlsx(cell, kind) for cell, kind in zip(row, kinds, strict=True)] for row in matrix
    ]


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


def _report_recipe(team: Sequence[Person], start: str, end: str) -> dict[str, JsonValue]:
    return {
        "subject": {"type": "people", "ids": list(_uuids(team))},
        "period": {"from": start, "to": end},
        "granularity": "quarter",
        "metric_keys": list(GOLDEN_FIELDS),
    }


@pytest.mark.requires_seed("dev_lead", "hr_lead")
@pytest.mark.parametrize("lead", ["dev_lead", "hr_lead"])
@pytest.mark.reliability
def test_a_lead_reads_the_whole_team_in_one_request_and_every_task_total_is_the_seeds(
    session_for: Callable[[str], PersonaSession], stand_manifest: Manifest, lead: str
) -> None:
    golden = _golden(stand_manifest)
    session = session_for(lead)
    team = _team(stand_manifest, lead)

    _task_metric_labels(session.client)
    _assert_subchart_holds(session, team)

    totals: defaultdict[tuple[str, str], float] = defaultdict(float)
    for start, end in _slices(golden):
        for cell, value in _period_values(session.client, _uuids(team), start, end).items():
            totals[cell] += value or 0

    for person in team:
        expected = golden.per_person.get(person.email)
        assert expected is not None, f"golden_metrics carries no entry for {person.email}"
        for key, field in GOLDEN_FIELDS.items():
            assert totals[key, person.uuid] == getattr(expected, field), (
                f"{lead}'s team view gives {person.email} {totals[key, person.uuid]} {key} "
                f"over {golden.window}; the seed wrote {getattr(expected, field)}"
            )


@pytest.mark.requires_seed("dev_lead", "hr_lead")
@pytest.mark.security
def test_a_team_request_naming_one_person_from_another_team_is_refused_whole(
    session_for: Callable[[str], PersonaSession], stand_manifest: Manifest
) -> None:
    start, end = query_window(stand_manifest)
    api = session_for("hr_lead").client
    own = _uuids(_team(stand_manifest, "hr_lead"))
    other = _uuids(_team(stand_manifest, "dev_lead"))

    answered = _ask_results(api, own, start, end)
    assert answered.status_code == 200, f"{answered.status_code} {answered.text[:300]}"
    answered.parse(MetricResultsResponse)

    for label, ids in (
        ("the development team", other),
        ("the HR team plus the development lead", [*own, other[0]]),
    ):
        refused = _ask_results(api, ids, start, end)
        assert refused.status_code == 403, (
            f"hr_lead asking for {label} answered {refused.status_code}: {refused.text[:300]}"
        )
        assert refused.parse(ProblemDocument).status == 403


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
