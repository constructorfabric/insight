from __future__ import annotations

import csv
import datetime as dt
import io
import zipfile
from collections.abc import Sequence
from xml.etree import ElementTree

import pytest
from insight_stand import ApiClient, ApiResponse, GoldenTasks, Manifest, Person, analytics_path

from ..schemas import MetricDefinitionListResponse, MetricResultsResponse, PeriodView
from . import MAX_QUERY_SPAN_DAYS

METRIC_DEFINITIONS = analytics_path("/v1/metric-definitions")
METRIC_RESULTS = analytics_path("/v1/metric-results")

CLOSED = "tasks.closed"
BUGS_FIXED = "tasks.bugs_fixed"
CLOSED_TASK = "tasks.closed_task"
GOLDEN_FIELDS = {CLOSED: "tasks_closed", BUGS_FIXED: "bugs_fixed", CLOSED_TASK: "closed_task"}

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
