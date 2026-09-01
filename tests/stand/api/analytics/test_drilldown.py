"""`/v1/metric-drilldown` and its export — the evidence behind a metric value.

    POST /v1/metric-drilldown         200 · 400 bad selection/cursor · 403 hidden · 404 unknown
    POST /v1/metric-drilldown/export  200 CSV/XLSX · 400 bad selection · 403 hidden · 404 unknown

The 415 half is in `test_request_contracts.py`, swept over every body route.

Two kinds of case live here. `git.commits` is the one metric exercised with a
filter, a display dimension and a one-row page limit, so the selection plumbing
and the cursor walk are covered somewhere concretely. Everything else is a sweep
over the whole catalogue, because drilldown capability is not declared per metric
— a source declares an evidence relation and a measure declares a granularity,
both mandatory — so the interesting failure is a whole evidence family drifting
from the observations derived from it, and no single-metric test can see that.

The sweep reconciles rather than smoke-tests: each metric's evidence must add up
to the value the dashboard shows for the same person and period. What "add up"
means per metric is `drilldown_matrix.py`, which also explains why a few metrics
only support an inequality.

Exports cover one metric per evidence presentation rather than the whole
catalogue: a presentation is all an export can differ by. `EXPORT_SHAPES` names
them, and the metrics not in it are deliberately not exported here. On top of
the shape sweep, `git.commits` is exported once more cell for cell: the seed
plants hostile commit titles (formula prefixes, an embedded tab and newline),
and the parity case asserts the export's actual neutralization contract
against the page the same selection returns.

The refusal catalogue (#1603 scenario 6) closes the file: every way a request
can be unservable — tampered pagination cursors included — is refused with a
reason a caller can tell from the others, and never with a partial page. The
export route must refuse an out-of-scope person exactly as the paged read does
(#1603 scenario 13), because a file download is the one response a caller
walks away with.
"""

from __future__ import annotations

import base64
import csv
import datetime as dt
import io
import json
import math
import statistics
import uuid
import warnings
import zipfile
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from typing import assert_never
from xml.etree import ElementTree

import pytest
from insight_stand import ApiClient, ApiResponse, Manifest, PersonaSession, analytics_path
from insight_stand.api import JsonValue

from ..schemas import (
    PROBLEM_CONTENT_TYPE,
    MetricDefinitionListResponse,
    MetricResultsResponse,
    PeriodView,
    ProblemDocument,
)
from ..schemas.analytics import (
    MetricDrilldownCapability,
    MetricDrilldownColumnType,
    MetricDrilldownEntity,
    MetricDrilldownEntity1,
    MetricDrilldownEntity3,
    MetricDrilldownResponse,
)
from . import query_window
from .drilldown_matrix import (
    EMPTY_EVIDENCE_METRIC,
    EVIDENCE_BLOCKED,
    EXPORT_SHAPES,
    MATRIX,
    EntityKind,
    Expectation,
    Tier,
)

DRILLDOWN = analytics_path("/v1/metric-drilldown")
DRILLDOWN_EXPORT = analytics_path("/v1/metric-drilldown/export")
METRIC_RESULTS = analytics_path("/v1/metric-results")
METRIC_DEFINITIONS = analytics_path("/v1/metric-definitions")
GIT_COMMITS = "git.commits"

#: The endpoint's own maximum, so the sweep walks a metric in as few requests as
#: the contract allows.
_PAGE_LIMIT = 250

#: Pages the sweep will walk before it stops asking. Reconciliation needs every
#: row, so a metric that exceeds this budget is reported as unreconciled rather
#: than reconciled against a prefix.
_PAGE_BUDGET = 40

#: At limit=1 the walk costs one round trip per evidence row, so this stays small.
_SINGLE_ROW_PAGE_BUDGET = 4

#: Aggregation order differs between the service (vectorized `sumIf` over
#: `Float64`) and this suite (row-ordered `sum`), and that is the only error
#: source — the transport is lossless both ways.
_REL_TOL = 1e-9
_ABS_TOL = 1e-9

#: Well-formed apart from the one field under test. An empty entity id is the
#: cheapest rejection that is unambiguously the HANDLER's — it needs no seeded
#: metric to reach, and no lookup can turn it into a 404 on the way.
_EMPTY_ENTITY_ID = ""

#: Together these send 150 values built from 50 distinct ones — past the
#: service's declared per-filter cap (`MAX_FILTER_VALUES` in the drilldown
#: domain's `dto.rs`) as sent, yet far under it once deduplicated, so only a
#: cap checked before dedup can refuse the request.
_FILTER_DISTINCT_VALUES = 50
_FILTER_VALUE_REPEATS = 3

#: A dimension no metric declares: `normalize_key` accepts the spelling, so the
#: refusal is attributable to the declaration check and nothing earlier.
_UNDECLARED_DIMENSION = "undeclared_dimension"


@dataclass(frozen=True)
class _Walk:
    """Every page of one selection, and whether the walk reached the end."""

    first: MetricDrilldownResponse
    rows: Sequence[Mapping[str, object]]
    complete: bool

    @property
    def column_keys(self) -> list[str]:
        return [column.key for column in self.first.columns]


@pytest.fixture(scope="session")
def drilldown_capabilities(
    lead_session: PersonaSession,
) -> Mapping[str, MetricDrilldownCapability | None]:
    """What the catalogue says each metric supports, read once for the sweep.

    Session-scoped because it is one answer for the whole run, and because the
    alternative — a listing request per parametrized case — would make the sweep
    cost twice what it needs to.

    Capability is derived per request from the health of the evidence relation,
    so this is a claim the endpoint can contradict, and it does: the catalogue's
    query requires the definition's schema to be checked as well, so it withholds
    the capability for metrics the endpoint serves. The sweep asserts the
    direction that holds — an advertised metric must answer — and
    `test_advertised_capability_matches_what_the_endpoint_serves` carries the
    other as a strict xfail.
    """
    response = lead_session.client.get(METRIC_DEFINITIONS)
    assert response.status_code == 200, f"definitions: {response.status_code}"
    metrics = response.parse(MetricDefinitionListResponse).metrics
    assert metrics, "no metric definitions — did the migrations run?"
    return {metric.metric_key: metric.drilldown for metric in metrics}


_EVIDENCE_BLOCKED_REASON = (
    "#2989: the CI source's evidence relation is marked unhealthy, so the endpoint refuses "
    "the drilldown and the catalogue withholds the capability"
)


def _marks_for(expectation: Expectation) -> tuple[pytest.MarkDecorator, ...]:
    """A metric the product refuses evidence for is a strict xfail, not a pass."""
    if expectation.metric_key not in EVIDENCE_BLOCKED:
        return ()
    return (pytest.mark.xfail(strict=True, reason=_EVIDENCE_BLOCKED_REASON),)


def _entity_body(manifest: Manifest, entity: EntityKind, entity_id: str | None) -> JsonValue:
    """The drilldown's entity block, person by default.

    A tenant entity carries no identifier: the route rejects a supplied one.
    """
    if entity == EntityKind.TENANT:
        return {"type": "tenant"}
    return {
        "type": "person",
        # Not `or`: an empty id is a case a caller may want to send, and
        # falling back to a real person would quietly test something else.
        "id": manifest.fixture("dev_lead").uuid if entity_id is None else entity_id,
    }


def _request_for(
    manifest: Manifest,
    metric_key: str,
    *,
    entity: EntityKind = EntityKind.PERSON,
    entity_id: str | None = None,
    limit: int | None = None,
    cursor: str | None = None,
    filters: Sequence[JsonValue] = (),
    display_dimensions: Sequence[str] = (),
) -> dict[str, JsonValue]:
    start, end = query_window(manifest)
    request: dict[str, JsonValue] = {
        "metric_key": metric_key,
        "entity": _entity_body(manifest, entity, entity_id),
        "period": {"from": start, "to": end},
        "filters": list(filters),
        "display_dimensions": list(display_dimensions),
    }
    if limit is not None:
        request["limit"] = limit
    if cursor is not None:
        request["cursor"] = cursor
    return request


def _seeded_request(
    manifest: Manifest,
    *,
    entity_id: str | None = None,
    limit: int | None = 1,
    cursor: str | None = None,
) -> dict[str, JsonValue]:
    return _request_for(
        manifest,
        GIT_COMMITS,
        entity_id=entity_id,
        limit=limit,
        cursor=cursor,
        filters=[{"dimension": "source", "values": ["github"]}],
        display_dimensions=["repository"],
    )


def _page(
    api: ApiClient,
    manifest: Manifest,
    metric_key: str,
    *,
    limit: int,
    entity: EntityKind = EntityKind.PERSON,
    cursor: str | None = None,
    filters: Sequence[JsonValue] = (),
    display_dimensions: Sequence[str] = (),
) -> ApiResponse:
    return api.post(
        DRILLDOWN,
        json_body=_request_for(
            manifest,
            metric_key,
            entity=entity,
            limit=limit,
            cursor=cursor,
            filters=filters,
            display_dimensions=display_dimensions,
        ),
    )


def _walk(
    api: ApiClient,
    manifest: Manifest,
    metric_key: str,
    *,
    limit: int,
    entity: EntityKind = EntityKind.PERSON,
    filters: Sequence[JsonValue] = (),
    display_dimensions: Sequence[str] = (),
    page_budget: int | None = None,
    initial: ApiResponse | None = None,
) -> _Walk:
    """Every page from `initial` (or a fresh first page) to the last or the budget."""
    cursors: set[str] = set()
    first: MetricDrilldownResponse | None = None
    rows: list[Mapping[str, object]] = []
    pages = 0
    response = initial or _page(
        api,
        manifest,
        metric_key,
        limit=limit,
        entity=entity,
        filters=filters,
        display_dimensions=display_dimensions,
    )

    while True:
        assert response.status_code == 200, (
            f"{metric_key}: status={response.status_code} {response.text[:300]}"
        )
        page = response.parse(MetricDrilldownResponse)
        if first is None:
            first = page
        else:
            assert page.columns == first.columns
            assert page.selection == first.selection
        rows.extend(row.values for row in page.rows)
        pages += 1
        cursor = page.next_cursor
        if cursor is None:
            break
        assert cursor not in cursors, f"repeated pagination cursor {cursor!r}"
        cursors.add(cursor)
        if page_budget is not None and pages >= page_budget:
            return _Walk(first=first, rows=rows, complete=False)
        response = _page(
            api,
            manifest,
            metric_key,
            limit=limit,
            entity=entity,
            cursor=cursor,
            filters=filters,
            display_dimensions=display_dimensions,
        )

    assert first is not None
    return _Walk(first=first, rows=rows, complete=True)


def _period_value(
    api: ApiClient,
    manifest: Manifest,
    metric_key: str,
    *,
    entity: EntityKind = EntityKind.PERSON,
    filters: Sequence[JsonValue] = (),
) -> float | None:
    """The scalar the dashboard shows for the same entity, period and filters.

    `None` is a real answer rather than a failure: nothing zero-fills, so an
    entity with no observations in the period has no value at all, and that is
    what an empty evidence page has to agree with.
    """
    start, end = query_window(manifest)
    tenant = entity == EntityKind.TENANT
    entity_id = manifest.tenant if tenant else manifest.fixture("dev_lead").uuid
    response = api.post(
        METRIC_RESULTS,
        json_body={
            "entity": {"type": "tenant"} if tenant else {"type": "person", "ids": [entity_id]},
            "period": {"from": start, "to": end},
            "metrics": [
                {
                    "metric_key": metric_key,
                    "filters": list(filters),
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
    assert values[0].entity_id == entity_id
    return values[0].value


def _export(api: ApiClient, request: dict[str, JsonValue], file_format: str) -> ApiResponse:
    request = dict(request)
    request.pop("limit", None)
    request["format"] = file_format
    return api.post(DRILLDOWN_EXPORT, json_body=request)


def _xlsx_rows(content: bytes) -> int:
    with zipfile.ZipFile(io.BytesIO(content)) as workbook:
        worksheet = ElementTree.fromstring(workbook.read("xl/worksheets/sheet1.xml"))
    return len(
        worksheet.findall(".//{http://schemas.openxmlformats.org/spreadsheetml/2006/main}row")
    )


_SSML = "{http://schemas.openxmlformats.org/spreadsheetml/2006/main}"

#: The CSV neutralization contract, read from `csv_safe_cell` in
#: `src/backend/services/analytics/src/domain/metric_drilldown/export.rs`: a
#: cell whose FIRST byte could start a spreadsheet formula (`=` `+` `-` `@`) or
#: shift the cell's content on paste (tab, CR, LF, space) is prefixed with a
#: single quote. Embedded occurrences are untouched — RFC 4180 quoting keeps
#: them inside the cell instead.
_CSV_NEUTRALIZED_PREFIXES = ("=", "+", "-", "@", "\t", "\r", "\n", " ")

#: What the seed plants on a handful of the dev lead's commit titles
#: (`HOSTILE_COMMIT_MESSAGES` in `src/ingestion/tools/seed/insight_seed/
#: generators/git.py`), named here only to prove the selection under test
#: actually contains each hostile class — never as an expectation of content.
_HOSTILE_PREFIXES = ("=", "+", "-", "@")
_HOSTILE_EMBEDDED = ("\t", "\n")


def _assert_csv_cell_matches(text: str, value: object, where: str) -> None:
    """One exported CSV cell against the paged value it must carry."""
    if value is None:
        assert text == "", f"{where}: null must export as an empty cell, got {text!r}"
    elif isinstance(value, bool):
        assert text == str(value).lower(), f"{where}: {text!r} is not {value}"
    elif isinstance(value, str):
        expected = f"'{value}" if value.startswith(_CSV_NEUTRALIZED_PREFIXES) else value
        assert text == expected, (
            f"{where}: exported {text!r}, expected {expected!r} — content must arrive intact, "
            "neutralized exactly per the export's own prefix rule"
        )
    else:
        assert isinstance(value, int | float), f"{where}: unexpected paged value {value!r}"
        # A negative number starts with `-`, so it is neutralized like text.
        bare = text.removeprefix("'")
        assert bare and _close(float(bare), float(value)), (
            f"{where}: exported {text!r} does not carry the paged number {value!r}"
        )


def _shared_strings(workbook: zipfile.ZipFile) -> list[str]:
    if "xl/sharedStrings.xml" not in workbook.namelist():
        return []
    root = ElementTree.fromstring(workbook.read("xl/sharedStrings.xml"))
    return [
        "".join(node.text or "" for node in item.iter(f"{_SSML}t"))
        for item in root.findall(f"{_SSML}si")
    ]


def _xlsx_column_index(reference: str) -> int:
    index = 0
    for character in reference:
        if character.isdigit():
            break
        index = index * 26 + (ord(character) - ord("A") + 1)
    return index - 1


def _xlsx_cell_matrix(content: bytes, width: int) -> list[list[str | float | bool | None]]:
    """Every worksheet cell decoded, with the inertness contract asserted.

    A formula cell would carry an `f` element (and a cached result typed
    `str`); asserting neither exists in ANY cell is what "stored as data, not
    as a formula" means at the file level. Text arrives as a shared or inline
    string, blanks as valueless cells, numbers and dates as numeric `v`.
    """
    with zipfile.ZipFile(io.BytesIO(content)) as workbook:
        strings = _shared_strings(workbook)
        sheet = ElementTree.fromstring(workbook.read("xl/worksheets/sheet1.xml"))
    matrix: list[list[str | float | bool | None]] = []
    for row in sheet.iter(f"{_SSML}row"):
        cells: list[str | float | bool | None] = [None] * width
        for cell in row.findall(f"{_SSML}c"):
            reference = cell.get("r", "")
            where = f"cell {reference or '<unaddressed>'}"
            assert cell.find(f"{_SSML}f") is None, f"{where}: exported as a formula"
            kind = cell.get("t", "n")
            assert kind != "str", f"{where}: typed as a formula's cached string result"
            index = _xlsx_column_index(reference)
            assert 0 <= index < width, f"{where}: outside the {width} exported columns"
            value_node = cell.find(f"{_SSML}v")
            if kind == "s":
                assert value_node is not None and value_node.text is not None, (
                    f"{where}: shared string without an index"
                )
                cells[index] = strings[int(value_node.text)]
            elif kind == "inlineStr":
                cells[index] = "".join(node.text or "" for node in cell.iter(f"{_SSML}t"))
            elif kind == "b":
                cells[index] = value_node is not None and value_node.text == "1"
            elif value_node is None or value_node.text is None:
                cells[index] = None
            else:
                cells[index] = float(value_node.text)
        matrix.append(cells)
    return matrix


#: `ExcelDateTime` serial numbers count days from this epoch (the 1900 date
#: system with its historical two-day offset), so a date cell decodes without
#: any expectation about which calendar day it holds.
_XLSX_EPOCH = dt.date(1899, 12, 30)


def _assert_xlsx_cell_matches(
    cell: str | float | bool | None,
    value: object,
    column_type: MetricDrilldownColumnType,
    where: str,
) -> None:
    """One decoded XLSX cell against the paged value it must carry."""
    if value is None:
        assert cell is None, f"{where}: null arrived as {cell!r}"
    elif column_type is MetricDrilldownColumnType.date and isinstance(value, str):
        assert isinstance(cell, float), f"{where}: date arrived as {cell!r}, not a serial"
        day = _XLSX_EPOCH + dt.timedelta(days=cell)
        assert day.isoformat() == value, f"{where}: serial {cell!r} decodes to {day}, not {value!r}"
    elif isinstance(value, bool):
        assert cell is value, f"{where}: boolean arrived as {cell!r}"
    elif isinstance(value, str):
        assert cell == value, (
            f"{where}: exported {cell!r}, expected {value!r} — XLSX applies no prefix, "
            "so the string must arrive byte-identical"
        )
    else:
        assert isinstance(value, int | float), f"{where}: unexpected paged value {value!r}"
        assert isinstance(cell, float) and _close(cell, float(value)), (
            f"{where}: exported {cell!r} does not carry the paged number {value!r}"
        )


def _numbers(rows: Sequence[Mapping[str, object]], column: str, metric_key: str) -> list[float]:
    values: list[float] = []
    for index, row in enumerate(rows):
        value = row[column]
        assert isinstance(value, int | float) and not isinstance(value, bool), (
            f"{metric_key}: row {index} column {column!r} is {value!r}, not a number"
        )
        values.append(float(value))
    return values


def _close(actual: float, expected: float) -> bool:
    return math.isclose(actual, expected, rel_tol=_REL_TOL, abs_tol=_ABS_TOL)


def _person_entity_id(entity: MetricDrilldownEntity) -> str:
    person = entity.root
    assert isinstance(person, MetricDrilldownEntity1), f"expected person entity, got {person.type}"
    return person.id


def _assert_selection_entity(
    entity: MetricDrilldownEntity, expectation: Expectation, person_id: str
) -> None:
    """The rows came back for the entity the request asked about.

    Narrowing the union is the assertion: a tenant metric answered under a
    person selection would be a different question answered.
    """
    if expectation.entity == EntityKind.TENANT:
        assert isinstance(entity.root, MetricDrilldownEntity3), (
            f"{expectation.metric_key}: expected the tenant selection, got {entity.root.type}"
        )
        return

    assert _person_entity_id(entity) == person_id


def _assert_shape(walk: _Walk, expectation: Expectation, person_id: str) -> None:
    selection = walk.first.selection
    assert selection.metric_key == expectation.metric_key
    _assert_selection_entity(selection.entity, expectation, person_id)
    assert selection.filters == []
    assert selection.display_dimensions == []

    keys = walk.column_keys
    assert "date" in keys, f"{expectation.metric_key}: no date column in {keys}"
    assert all(set(row) == set(keys) for row in walk.rows), (
        f"{expectation.metric_key}: a row's keys disagree with {keys}"
    )

    match expectation.tier:
        case Tier.EXACT_COUNT:
            assert "value" not in keys, (
                f"{expectation.metric_key}: a counted metric projects no value column, got {keys}"
            )
        case Tier.DERIVED_MEDIAN:
            assert "value" not in keys, (
                f"{expectation.metric_key}: unexpected value column in {keys}"
            )
            for column in expectation.derived_from:
                assert column in keys, f"{expectation.metric_key}: {column!r} missing from {keys}"
        case Tier.EXACT_RATIO | Tier.COLLAPSE_BOUNDED_RATIO:
            assert "numerator" in keys and "denominator" in keys, (
                f"{expectation.metric_key}: a ratio projects both sides, got {keys}"
            )
            assert "value" not in keys, (
                f"{expectation.metric_key}: unexpected value column in {keys}"
            )
        case (
            Tier.EXACT_SUM
            | Tier.EXACT_MEDIAN
            | Tier.EXACT_PERCENTILE
            | Tier.EXACT_STDDEV
            | Tier.EXACT_DISTINCT_DATES
            | Tier.COLLAPSE_BOUNDED_SUM
            | Tier.STRUCTURAL_ONLY
        ):
            assert "value" in keys, f"{expectation.metric_key}: no value column in {keys}"
        case unhandled:
            # A tier added without a shape rule would otherwise assert nothing.
            assert_never(unhandled)


def _assert_median(period: float | None, values: Sequence[float], metric_key: str) -> None:
    """`quantileExact(0.5)` returns a stored element, so this is an identity.

    Both middle elements are accepted rather than one: which of them the server
    returns for an even count is its own tie rule, and pinning it here would test
    ClickHouse rather than the evidence.
    """
    ordered = sorted(values)
    middle = {ordered[(len(ordered) - 1) // 2], ordered[len(ordered) // 2]}
    assert period in middle, (
        f"{metric_key}: period {period} is neither middle value of {len(ordered)} "
        f"evidence rows {sorted(middle)}"
    )


def _assert_percentile(
    period: float | None, values: Sequence[float], quantile: float, metric_key: str
) -> None:
    """`quantileExact(p)` returns a stored element, so this is an identity too.

    The element sits at index `floor(p x n)` of the sorted values. Its
    neighbour below is accepted as well, for the same reason `_assert_median`
    accepts both middle values: which side of a tie the server takes is its own
    rule, and pinning it here would test ClickHouse rather than the evidence.
    """
    ordered = sorted(values)
    index = min(len(ordered) - 1, int(quantile * len(ordered)))
    candidates = {ordered[max(0, index - 1)], ordered[index]}
    assert period in candidates, (
        f"{metric_key}: period {period} is not the p{quantile:.0%} element of "
        f"{len(ordered)} evidence rows, nor its neighbour {sorted(candidates)}"
    )


def _assert_ratio(
    period: float | None, walk: _Walk, expectation: Expectation, *, bounded: bool
) -> None:
    """A ratio reconciles as summed-numerator over summed-denominator.

    Two degenerate cases are the endpoint's, not this test's. An all-zero
    denominator has no ratio, so the metric is null. A numerator with no rows is
    also null, while the drilldown still returns a row per day whose numerator
    reads `0` — the evidence cannot tell "absent" from "zero" and the metric
    deliberately can, so the only safe statement there is null-or-transformed-zero.
    """
    numerator = sum(_numbers(walk.rows, "numerator", expectation.metric_key))
    denominator = sum(_numbers(walk.rows, "denominator", expectation.metric_key))
    assert expectation.scale is not None, f"{expectation.metric_key}: ratio without a scale"
    transform = expectation.transform

    if denominator == 0:
        assert period is None, (
            f"{expectation.metric_key}: denominator sums to zero but the metric answered {period}"
        )
        return
    if numerator == 0:
        zero = transform.apply(0.0) if transform else 0.0
        assert period is None or _close(period, zero), (
            f"{expectation.metric_key}: numerator sums to zero, so the metric is null or {zero}, "
            f"and it answered {period}"
        )
        return

    evidence = expectation.scale * numerator / denominator
    if bounded:
        assert period is not None and (period > evidence or _close(period, evidence)), (
            f"{expectation.metric_key}: evidence ratio {evidence} exceeds the metric's {period}; "
            "day flags collapse to at most one row per person per day, so the metric can only be "
            "the larger of the two"
        )
        return

    expected = transform.apply(evidence) if transform else evidence
    assert period is not None and _close(period, expected), (
        f"{expectation.metric_key}: evidence gives {expected} ({numerator}/{denominator} "
        f"scaled by {expectation.scale}) but the metric answered {period}"
    )


def _assert_stddev(period: float | None, values: Sequence[float], metric_key: str) -> None:
    """`stddevSampIf` is the sample deviation, so the evidence recomputes it.

    A single row has no sample deviation to speak of — ClickHouse answers nan
    there — so the one-row case asserts only that the metric declined to claim
    one, rather than pinning a value neither side defines.
    """
    if len(values) < 2:
        assert period is None or math.isnan(period), (
            f"{metric_key}: {len(values)} evidence row(s) cannot have a sample deviation, "
            f"but the metric answered {period}"
        )
        return

    deviation = statistics.stdev(values)
    assert period is not None and _close(deviation, period), (
        f"{metric_key}: evidence deviates by {deviation} over {len(values)} rows "
        f"against a metric value of {period}"
    )


def _reconcile(period: float | None, walk: _Walk, expectation: Expectation) -> None:
    metric_key = expectation.metric_key

    match expectation.tier:
        case Tier.EXACT_COUNT:
            assert len(walk.rows) == period, (
                f"{metric_key}: {len(walk.rows)} evidence rows against a metric value of {period}"
            )
        case Tier.EXACT_SUM:
            total = sum(_numbers(walk.rows, "value", metric_key))
            assert period is not None and _close(total, period), (
                f"{metric_key}: evidence sums to {total} against a metric value of {period}"
            )
        case Tier.EXACT_MEDIAN:
            _assert_median(period, _numbers(walk.rows, "value", metric_key), metric_key)
        case Tier.EXACT_PERCENTILE:
            assert expectation.quantile is not None, (
                f"{metric_key}: an EXACT_PERCENTILE expectation carries no quantile"
            )
            _assert_percentile(
                period,
                _numbers(walk.rows, "value", metric_key),
                expectation.quantile,
                metric_key,
            )
        case Tier.EXACT_STDDEV:
            _assert_stddev(period, _numbers(walk.rows, "value", metric_key), metric_key)
        case Tier.DERIVED_MEDIAN:
            parts = [_numbers(walk.rows, column, metric_key) for column in expectation.derived_from]
            _assert_median(period, [sum(row) for row in zip(*parts, strict=True)], metric_key)
        case Tier.EXACT_RATIO:
            _assert_ratio(period, walk, expectation, bounded=False)
        case Tier.COLLAPSE_BOUNDED_RATIO:
            _assert_ratio(period, walk, expectation, bounded=True)
        case Tier.EXACT_DISTINCT_DATES:
            dates = {row["date"] for row in walk.rows}
            assert len(dates) == period, (
                f"{metric_key}: {len(dates)} distinct evidence dates against a metric value "
                f"of {period}"
            )
        case Tier.COLLAPSE_BOUNDED_SUM:
            total = sum(_numbers(walk.rows, "value", metric_key))
            assert period is not None and (total > period or _close(total, period)), (
                f"{metric_key}: evidence sums to {total}, below the metric's {period}; a day flag "
                "collapses across a person's accounts, so evidence can only be the larger side"
            )
        case Tier.STRUCTURAL_ONLY:
            assert period is not None and 1 <= period <= len(walk.rows), (
                f"{metric_key}: a distinct count over {len(walk.rows)} evidence rows cannot "
                f"be {period}"
            )
        case unhandled:
            # A tier added without a reconciliation would otherwise pass silently.
            assert_never(unhandled)


def _assert_evidence_unavailable(
    response: ApiResponse, metric_key: str, capability: MetricDrilldownCapability | None
) -> None:
    """A refusal has to be the documented one, and the catalogue has to agree.

    Only this direction is sound. The endpoint refuses when the evidence relation
    is unhealthy, and the catalogue's capability query requires everything that
    refusal tests plus more, so a refused metric can never be an advertised one.
    The converse does NOT hold — see
    `test_advertised_capability_matches_what_the_endpoint_serves`.
    """
    assert response.status_code == 400, (
        f"{metric_key}: expected the documented refusal, got {response.status_code}: "
        f"{response.text[:300]}"
    )
    assert response.parse(ProblemDocument).status == 400
    assert "EVIDENCE_UNAVAILABLE" in response.text, (
        f"{metric_key}: refused without naming the precondition: {response.text[:300]}"
    )
    assert capability is None, (
        f"{metric_key}: the catalogue advertises drilldown for it, so a reader is offered "
        "supporting data the endpoint then refuses to serve"
    )


@pytest.mark.versatility
def test_every_metric_definition_is_in_the_drilldown_matrix(
    drilldown_capabilities: Mapping[str, MetricDrilldownCapability | None],
) -> None:
    """The sweep's denominator, pinned against the catalogue the stand serves.

    Every metric is drilldown-declared by construction, so a metric added
    without an entry here would silently narrow the sweep instead of failing it.
    Compared as sets rather than counted: a count would only say that the two
    disagree, and the metric key is what a reader needs.
    """
    expected = {expectation.metric_key for expectation in MATRIX}
    served = set(drilldown_capabilities)
    assert served == expected, (
        f"served but unexpected: {sorted(served - expected)}; "
        f"expected but not served: {sorted(expected - served)}"
    )


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.xfail(
    strict=True,
    reason="the catalogue withholds the capability for a metric whose definition schema_status "
    "is not ok, while the drilldown endpoint serves that metric's evidence, so the UI hides "
    "supporting data that exists",
)
@pytest.mark.reliability
def test_advertised_capability_matches_what_the_endpoint_serves(
    api: ApiClient,
    stand_manifest: Manifest,
    drilldown_capabilities: Mapping[str, MetricDrilldownCapability | None],
) -> None:
    """The half of the capability contract that does not hold yet.

    `GET /v1/metric-definitions` is what the UI reads to decide whether to offer
    "supporting data" at all, so a metric the endpoint would answer but the
    catalogue calls incapable has evidence no reader can reach. One metric per
    evidence presentation is asked, which is enough to catch a family-wide
    difference without a second sweep over the catalogue.

    Strict, so the day the two sides agree this fails as an XPASS and the marker
    comes off rather than quietly staying.
    """
    hidden = sorted(
        metric_key
        for metric_key in EXPORT_SHAPES
        if drilldown_capabilities.get(metric_key) is None
        and _page(api, stand_manifest, metric_key, limit=1).status_code == 200
    )
    assert hidden == [], (
        f"the endpoint serves evidence for {hidden}, and the catalogue advertises none, "
        "so the affordance is hidden for metrics that have supporting data"
    )


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.parametrize(
    "expectation",
    [pytest.param(expectation, marks=_marks_for(expectation)) for expectation in MATRIX],
    ids=[expectation.metric_key for expectation in MATRIX],
)
@pytest.mark.reliability
def test_drilldown_reconciles_with_the_metric_value(
    api: ApiClient,
    stand_manifest: Manifest,
    drilldown_capabilities: Mapping[str, MetricDrilldownCapability | None],
    expectation: Expectation,
) -> None:
    """One metric's evidence against the value the dashboard shows for it.

    No filter and no display dimension, deliberately. A ratio's two measures
    need not carry the same dimensions, so filtering on one of them can empty
    the denominator on the metric side while the evidence side still returns
    rows — a difference in the request, read as a difference in the data. The
    filtered path is covered concretely by the `git.commits` cases above.

    An empty page is a legitimate answer, and it is still an assertion: nothing
    zero-fills, so evidence and metric have to be empty together.

    Driven off what the endpoint answers rather than off the advertised
    capability, because the two do not agree today and the endpoint is the side
    that decides whether evidence exists.
    """
    person_id = stand_manifest.fixture("dev_lead").uuid
    probe = _page(
        api,
        stand_manifest,
        expectation.metric_key,
        limit=_PAGE_LIMIT,
        entity=expectation.entity,
    )
    if expectation.metric_key in EVIDENCE_BLOCKED:
        assert probe.status_code == 200, (
            f"{expectation.metric_key}: evidence exists but the endpoint refused it — "
            f"{probe.status_code}: {probe.text[:300]}"
        )
    if probe.status_code != 200:
        _assert_evidence_unavailable(
            probe, expectation.metric_key, drilldown_capabilities.get(expectation.metric_key)
        )
        return

    walk = _walk(
        api,
        stand_manifest,
        expectation.metric_key,
        limit=_PAGE_LIMIT,
        entity=expectation.entity,
        page_budget=_PAGE_BUDGET,
        initial=probe,
    )
    _assert_shape(walk, expectation, person_id)

    period = _period_value(api, stand_manifest, expectation.metric_key, entity=expectation.entity)
    if not walk.rows:
        assert period is None, (
            f"{expectation.metric_key}: no evidence rows, but the metric answered {period}"
        )
        return

    if not walk.complete:
        warnings.warn(
            f"{expectation.metric_key}: stopped after {_PAGE_BUDGET} pages of "
            f"{_PAGE_LIMIT} rows, so its value was NOT reconciled — only the page shape was",
            stacklevel=1,
        )
        return

    _reconcile(period, walk, expectation)


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.parametrize("metric_key", EXPORT_SHAPES)
@pytest.mark.reliability
def test_drilldown_export_carries_every_row(
    api: ApiClient,
    stand_manifest: Manifest,
    metric_key: str,
) -> None:
    """Both export formats, against the page the same selection returns.

    The header is the column LABELS while the page carries keys, so a serializer
    that lost the labelling would still round-trip its own output — comparing
    against the page is what catches it. The empty case is in here too: an export
    of no evidence is a header and nothing else, not an error.
    """
    walk = _walk(api, stand_manifest, metric_key, limit=_PAGE_LIMIT, page_budget=_PAGE_BUDGET)
    assert walk.complete, f"{metric_key}: export shapes must be small enough to walk whole"
    request = _request_for(stand_manifest, metric_key)

    csv_response = _export(api, request, "csv")
    assert csv_response.status_code == 200, f"{metric_key}: {csv_response.text[:300]}"
    assert csv_response.content_type.startswith("text/csv")
    assert ".csv" in csv_response.headers.get("content-disposition", "")
    csv_rows = list(csv.reader(io.StringIO(csv_response.content.decode("utf-8-sig"))))
    assert csv_rows[0] == [column.label for column in walk.first.columns]
    assert len(csv_rows) == len(walk.rows) + 1

    xlsx_response = _export(api, request, "xlsx")
    assert xlsx_response.status_code == 200, f"{metric_key}: {xlsx_response.text[:300]}"
    assert xlsx_response.content_type == (
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    )
    assert ".xlsx" in xlsx_response.headers.get("content-disposition", "")
    assert _xlsx_rows(xlsx_response.content) == len(walk.rows) + 1


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.versatility
def test_export_cells_match_the_page_and_hostile_values_stay_inert(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    """#1603 scenario 11 — export cell parity and escaping.

    The count sweep above proves nothing about content, and content is where
    an export can betray its reader: a commit title is attacker-shaped free
    text, and a spreadsheet will happily execute one that starts with `=`. The
    seed plants clearly synthetic titles covering each formula prefix plus an
    embedded tab and newline, so the `git.commits` selection is the one whose
    export has something to get wrong.

    Both formats are compared cell for cell against the page the same
    selection returns — both sides of every comparison come from the service.
    The escaping asserted is the backend's actual contract (`csv_safe_cell`):
    a CSV cell's first byte in the prefix set earns a leading single quote,
    everything else round-trips byte-identical, and RFC 4180 quoting keeps
    embedded tabs and newlines inside their cell, so the row count is the
    page's. XLSX applies no prefix at all: every value arrives byte-identical
    as a shared or inline string, and no cell is a formula.
    """
    walk = _walk(api, stand_manifest, GIT_COMMITS, limit=_PAGE_LIMIT, page_budget=_PAGE_BUDGET)
    assert walk.complete, "the hostile selection must be small enough to walk whole"
    keys = walk.column_keys
    assert "title" in keys, f"no title column in {keys} — nowhere for a hostile value to surface"

    titles = [row["title"] for row in walk.rows if isinstance(row["title"], str)]
    for prefix in _HOSTILE_PREFIXES:
        assert any(title.startswith(prefix) for title in titles), (
            f"no evidence title begins with {prefix!r} — the stand predates the hostile-title "
            "seed, so this test would prove nothing; re-seed it"
        )
    for embedded in _HOSTILE_EMBEDDED:
        assert any(embedded in title for title in titles), (
            f"no evidence title embeds {embedded!r} — the stand predates the hostile-title "
            "seed, so this test would prove nothing; re-seed it"
        )

    request = _request_for(stand_manifest, GIT_COMMITS)

    csv_response = _export(api, request, "csv")
    assert csv_response.status_code == 200, f"csv: {csv_response.text[:300]}"
    csv_rows = list(csv.reader(io.StringIO(csv_response.content.decode("utf-8-sig"))))
    assert csv_rows[0] == [column.label for column in walk.first.columns]
    assert len(csv_rows) == len(walk.rows) + 1, (
        f"CSV parsed to {len(csv_rows) - 1} rows against {len(walk.rows)} paged rows — "
        "an embedded newline escaped its cell"
    )
    for row_index, (page_row, csv_row) in enumerate(zip(walk.rows, csv_rows[1:], strict=True)):
        assert len(csv_row) == len(keys), (
            f"CSV row {row_index} has {len(csv_row)} cells for {len(keys)} columns — "
            "an embedded delimiter escaped its cell"
        )
        for key, text in zip(keys, csv_row, strict=True):
            _assert_csv_cell_matches(text, page_row[key], f"CSV row {row_index} column {key!r}")

    xlsx_response = _export(api, request, "xlsx")
    assert xlsx_response.status_code == 200, f"xlsx: {xlsx_response.text[:300]}"
    matrix = _xlsx_cell_matrix(xlsx_response.content, len(keys))
    assert len(matrix) == len(walk.rows) + 1
    assert matrix[0] == [column.label for column in walk.first.columns]
    for row_index, (page_row, sheet_row) in enumerate(zip(walk.rows, matrix[1:], strict=True)):
        for column_index, column in enumerate(walk.first.columns):
            _assert_xlsx_cell_matches(
                sheet_row[column_index],
                page_row[column.key],
                column.type,
                f"XLSX row {row_index} column {column.key!r}",
            )


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_git_commit_drilldown_pages_and_reconciles(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    walk = _walk(
        api,
        stand_manifest,
        GIT_COMMITS,
        limit=_PAGE_LIMIT,
        filters=[{"dimension": "source", "values": ["github"]}],
        display_dimensions=["repository"],
    )
    person_id = stand_manifest.fixture("dev_lead").uuid

    assert walk.first.selection.metric_key == GIT_COMMITS
    assert _person_entity_id(walk.first.selection.entity) == person_id
    assert walk.first.selection.display_dimensions == ["repository"]
    assert [(item.dimension, item.values) for item in walk.first.selection.filters] == [
        ("source", ["github"])
    ]
    column_keys = walk.column_keys
    assert "repository" in column_keys
    assert "date" in column_keys
    assert walk.rows
    assert all(set(row) == set(column_keys) for row in walk.rows)
    assert len(walk.rows) == _period_value(
        api,
        stand_manifest,
        GIT_COMMITS,
        filters=[{"dimension": "source", "values": ["github"]}],
    )


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_git_commit_drilldown_pages_a_row_at_a_time(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    """Consecutive one-row pages neither repeat nor skip a row.

    Compared against the prefix of the bulk walk: `_walk` on its own catches a
    repeated cursor, never a dropped or duplicated row.
    """
    filters: Sequence[JsonValue] = [{"dimension": "source", "values": ["github"]}]
    single = _walk(
        api,
        stand_manifest,
        GIT_COMMITS,
        limit=1,
        filters=filters,
        display_dimensions=["repository"],
        page_budget=_SINGLE_ROW_PAGE_BUDGET,
    )
    bulk = _walk(
        api,
        stand_manifest,
        GIT_COMMITS,
        limit=_PAGE_LIMIT,
        filters=filters,
        display_dimensions=["repository"],
    )

    assert len(single.rows) == _SINGLE_ROW_PAGE_BUDGET, (
        "the seeded evidence set is shorter than the budget, so this walk never "
        "paginated and proves nothing about the cursor"
    )
    assert single.rows == bulk.rows[:_SINGLE_ROW_PAGE_BUDGET]


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_git_commit_drilldown_exports_all_rows(api: ApiClient, stand_manifest: Manifest) -> None:
    walk = _walk(
        api,
        stand_manifest,
        GIT_COMMITS,
        limit=_PAGE_LIMIT,
        filters=[{"dimension": "source", "values": ["github"]}],
        display_dimensions=["repository"],
    )
    request = _seeded_request(stand_manifest, limit=None)

    csv_response = _export(api, request, "csv")
    assert csv_response.status_code == 200
    assert csv_response.content_type.startswith("text/csv")
    assert ".csv" in csv_response.headers.get("content-disposition", "")
    csv_rows = list(csv.reader(io.StringIO(csv_response.content.decode("utf-8-sig"))))
    assert csv_rows[0] == [column.label for column in walk.first.columns]
    assert len(csv_rows) == len(walk.rows) + 1

    xlsx_response = _export(api, request, "xlsx")
    assert xlsx_response.status_code == 200
    assert xlsx_response.content_type == (
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    )
    assert ".xlsx" in xlsx_response.headers.get("content-disposition", "")
    assert _xlsx_rows(xlsx_response.content) == len(walk.rows) + 1


@pytest.mark.requires_seed("dev_lead", "sales_ic")
@pytest.mark.security
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
@pytest.mark.reliability
def test_supported_metric_with_no_evidence_returns_an_empty_page(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    request = _seeded_request(stand_manifest)
    request["metric_key"] = EMPTY_EVIDENCE_METRIC
    request["filters"] = []
    request["display_dimensions"] = []

    response = api.post(DRILLDOWN, json_body=request)
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
    result = response.parse(MetricDrilldownResponse)
    assert result.selection.metric_key == EMPTY_EVIDENCE_METRIC
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


@pytest.mark.reliability
def test_drilldown_400_empty_entity_id(api: ApiClient) -> None:
    response = api.post(DRILLDOWN, json_body=_request())
    assert response.status_code == 400, f"status={response.status_code} {response.text[:300]}"


@pytest.mark.reliability
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
@pytest.mark.reliability
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


def _refusal(response: ApiResponse, status: int, *needles: str) -> ProblemDocument:
    """The response as the bare problem document it must be, and nothing else.

    `ProblemDocument` forbids extra fields, so a body that carried rows or a
    `next_cursor` alongside the error — a partial response — fails the parse
    instead of passing as a refusal. The needles are each class's own words in
    the document, which is what makes one refusal distinguishable from the
    next; a needle-less call asserts only the envelope.
    """
    assert response.status_code == status, (
        f"expected {status}, got {response.status_code}: {response.text[:300]}"
    )
    document = response.parse(ProblemDocument)
    assert document.status == status
    for needle in needles:
        assert needle in response.text, (
            f"refused, but not for the asserted reason {needle!r}: {response.text[:300]}"
        )
    return document


def _send(api: ApiClient, path: str, request: dict[str, JsonValue]) -> ApiResponse:
    """`request` as-is to the paged route, reshaped by `_export` for the other."""
    if path == DRILLDOWN_EXPORT:
        return _export(api, request, "csv")
    return api.post(path, json_body=request)


@pytest.fixture(scope="session")
def issued_cursor(lead_session: PersonaSession, stand_manifest: Manifest) -> str:
    """A pagination cursor the service genuinely issued, for tampering with.

    Each tampering case flips exactly one envelope field of a real cursor, so
    its refusal is attributable to that field rather than to a cursor the
    service would never have produced. Session-scoped: one issued cursor
    serves every case, and issuing one costs a full drilldown request.
    """
    response = lead_session.client.post(DRILLDOWN, json_body=_seeded_request(stand_manifest))
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
    cursor = response.parse(MetricDrilldownResponse).next_cursor
    assert cursor is not None, (
        "the seeded git.commits selection fit one page at limit=1, so no cursor was issued "
        "and the tampering cases have nothing genuine to start from"
    )
    return cursor


def _tampered(cursor: str, **overrides: JsonValue) -> str:
    """A genuinely issued cursor, re-encoded with named envelope fields replaced.

    Both halves mirror the service's `cursor.rs`: a url-safe unpadded base64
    JSON envelope. Asserting the field exists before overriding keeps a
    backend rename from silently turning a tamper into an ignored extra field
    — the test would then refuse for the wrong reason and still pass.
    """
    envelope = json.loads(base64.urlsafe_b64decode(cursor + "=" * (-len(cursor) % 4)))
    assert isinstance(envelope, dict), f"cursor envelope is not an object: {envelope!r}"
    for field in overrides:
        assert field in envelope, f"cursor envelope has no {field!r}: {sorted(envelope)}"
    envelope.update(overrides)
    return base64.urlsafe_b64encode(json.dumps(envelope).encode()).decode().rstrip("=")


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.parametrize(
    ("label", "cursor"),
    [
        ("not base64", "@@not-a-cursor@@"),
        ("base64 of non-JSON", base64.urlsafe_b64encode(b"not json").decode().rstrip("=")),
        ("JSON without the envelope fields", base64.urlsafe_b64encode(b"{}").decode().rstrip("=")),
    ],
)
@pytest.mark.reliability
def test_drilldown_refuses_a_malformed_cursor(
    api: ApiClient, stand_manifest: Manifest, label: str, cursor: str
) -> None:
    """#1603 scenario 6 — a cursor that never was one is refused as malformed.

    All three spellings — undecodable, decodable to non-JSON, JSON missing the
    envelope's fields — are one class: nothing here was ever issued. The reason
    is the class's own words, so a caller can tell a corrupted cursor from one
    that outlived its snapshot, which asks for a restart rather than a bug
    report.
    """
    response = api.post(DRILLDOWN, json_body=_seeded_request(stand_manifest, cursor=cursor))
    _refusal(response, 400, "cursor is malformed")


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_drilldown_refuses_a_wrong_version_cursor(
    api: ApiClient, stand_manifest: Manifest, issued_cursor: str
) -> None:
    """#1603 scenario 6 — a genuine cursor with its version flipped is refused.

    The version field is the envelope's compatibility escape hatch. Flipping it
    on an otherwise genuine cursor pins that the service reads the field rather
    than accepting whatever decodes — and the reason names the version, not
    malformedness, so a cursor from a different deployment generation is
    distinguishable from corruption.

    Version `0` rather than "the next one": a version the service has not
    reached yet becomes the one it serves at the next bump, and this test then
    asserts a refusal of the cursor the service now issues. Versions only ever
    count up, so `0` is the one value that can never be current.
    """
    response = api.post(
        DRILLDOWN,
        json_body=_seeded_request(stand_manifest, cursor=_tampered(issued_cursor, version=0)),
    )
    _refusal(response, 400, "cursor version is unsupported")


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_drilldown_refuses_a_cursor_replayed_against_another_selection(
    api: ApiClient, stand_manifest: Manifest, issued_cursor: str
) -> None:
    """#1603 scenario 6 — a cursor is bound to the selection that issued it.

    The replay keeps the metric — so the evidence relation and its snapshot
    stay valid — and drops the filter and display dimension the cursor was
    issued under, leaving the selection fingerprint as the only check that can
    fire. That is the point: accepting the cursor would resume a filtered walk
    inside an unfiltered one and quietly skip every row the old page ordering
    had already passed.

    A replay against a different METRIC is refused too, but lands as
    `EVIDENCE_SNAPSHOT_EXPIRED` whenever the two evidence relations differ,
    because the snapshot check runs before the fingerprint comparison — this
    case keeps the metric so the fingerprint refusal itself is pinned.
    """
    request = _request_for(stand_manifest, GIT_COMMITS, limit=1, cursor=issued_cursor)
    response = api.post(DRILLDOWN, json_body=request)
    _refusal(response, 400, "cursor does not match the metric selection")


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_drilldown_refuses_a_cursor_from_an_expired_snapshot(
    api: ApiClient, stand_manifest: Manifest, issued_cursor: str
) -> None:
    """#1603 scenario 6 — a snapshot no longer the table's is a failed precondition.

    `snapshot_id` is the evidence table's UUID, so a genuine cursor with a
    random one swapped in is exactly what a caller holds after the evidence
    was rebuilt mid-walk. The documented refusal is the precondition violation
    `EVIDENCE_SNAPSHOT_EXPIRED`, not an invalid argument: the cursor was
    well-formed and honestly held, the world moved on. Its reason code is what
    tells a client "restart the walk" apart from "your cursor is garbage".
    """
    tampered = _tampered(issued_cursor, snapshot_id=str(uuid.uuid4()))
    response = api.post(DRILLDOWN, json_body=_seeded_request(stand_manifest, cursor=tampered))
    _refusal(response, 400, "EVIDENCE_SNAPSHOT_EXPIRED")


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.parametrize("path", [DRILLDOWN, DRILLDOWN_EXPORT], ids=["drilldown", "export"])
@pytest.mark.reliability
def test_drilldown_refuses_an_undeclared_filter_dimension(
    api: ApiClient, stand_manifest: Manifest, path: str
) -> None:
    """#1603 scenario 6 — a filter on a dimension the metric never declared.

    Passing it through instead would make the filter a probe into the evidence
    relation's real columns. The refusal names the `filters.dimension` field,
    which is what separates it from the same words said about a display
    dimension.
    """
    request = _request_for(
        stand_manifest,
        GIT_COMMITS,
        filters=[{"dimension": _UNDECLARED_DIMENSION, "values": ["anything"]}],
    )
    response = _send(api, path, request)
    _refusal(response, 400, "filters.dimension", "is not declared by the metric")


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.parametrize("path", [DRILLDOWN, DRILLDOWN_EXPORT], ids=["drilldown", "export"])
@pytest.mark.reliability
def test_drilldown_refuses_a_duplicated_filter_dimension(
    api: ApiClient, stand_manifest: Manifest, path: str
) -> None:
    """#1603 scenario 6 — the same dimension filtered twice is refused, not merged.

    Two filters on one dimension have no single meaning — intersection empties
    the page, union widens it — and either silent choice would be read as a
    statement about the data. Both value lists are individually valid, so the
    refusal is attributable to the duplication alone.
    """
    request = _request_for(
        stand_manifest,
        GIT_COMMITS,
        filters=[
            {"dimension": "source", "values": ["github"]},
            {"dimension": "source", "values": ["gitlab"]},
        ],
    )
    response = _send(api, path, request)
    _refusal(response, 400, "duplicate dimension filter")


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.parametrize("path", [DRILLDOWN, DRILLDOWN_EXPORT], ids=["drilldown", "export"])
@pytest.mark.reliability
def test_drilldown_refuses_a_filter_over_the_declared_value_cap(
    api: ApiClient, stand_manifest: Manifest, path: str
) -> None:
    """#1603 scenario 6 — a value list past the per-filter cap is refused up front.

    The cap is checked before values are deduplicated, so the refusal is about
    the request's size as sent — a caller cannot smuggle an oversized list past
    it with repeats, and the query never compiles an IN-list this long. The
    list is deliberately repeats of a few distinct values: it would dedup to
    well under the cap, so a 400 here pins the before-dedup ordering rather
    than passing for either.
    """
    values: list[JsonValue] = [
        f"value_{index:03d}" for index in range(_FILTER_DISTINCT_VALUES)
    ] * _FILTER_VALUE_REPEATS
    request = _request_for(
        stand_manifest,
        GIT_COMMITS,
        filters=[{"dimension": "source", "values": values}],
    )
    response = _send(api, path, request)
    _refusal(response, 400, "filters.values", "between 1 and 100 values are required")


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.parametrize("path", [DRILLDOWN, DRILLDOWN_EXPORT], ids=["drilldown", "export"])
@pytest.mark.reliability
def test_drilldown_refuses_an_undeclared_display_dimension(
    api: ApiClient, stand_manifest: Manifest, path: str
) -> None:
    """#1603 scenario 6 — a display dimension the metric never declared.

    A projected column reaches the response (and the export file), so an
    unchecked one would be a column probe with the answer written into the
    page. The refusal names `display_dimensions`, distinguishing it from the
    filter-side twin of the same message.
    """
    request = _request_for(stand_manifest, GIT_COMMITS, display_dimensions=[_UNDECLARED_DIMENSION])
    response = _send(api, path, request)
    _refusal(response, 400, "display_dimensions", "is not declared by the metric")


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.parametrize("path", [DRILLDOWN, DRILLDOWN_EXPORT], ids=["drilldown", "export"])
@pytest.mark.reliability
def test_drilldown_refuses_an_unknown_metric_key(
    api: ApiClient, stand_manifest: Manifest, path: str
) -> None:
    """#1603 scenario 6 — a well-formed key that names no metric is a 400.

    The same classification `/v1/metric-results` uses: the catalogue loader
    refuses a key it does not carry as an `UNAVAILABLE` field violation before
    the drilldown validation's own not-found fallback can fire, so on the wire
    an unknown key is a 400 on both operations. The key is shaped to pass
    `normalize_metric_key`, so the refusal is the catalogue's and not a
    spelling rejection dressed as one.
    """
    request = _request_for(stand_manifest, "stand.does_not_exist")
    response = _send(api, path, request)
    _refusal(response, 400, "unknown or unavailable metric key")


@pytest.mark.requires_seed("dev_lead", "sales_ic")
@pytest.mark.security
def test_drilldown_export_refuses_an_out_of_scope_person_identically(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    """#1603 scenario 13 — both export formats refuse exactly as the paged read.

    Same outsider and selection as
    `test_git_commit_drilldown_refuses_a_person_out_of_scope`. An export that
    answered differently — a softer status, an empty file, or a filename via
    content-disposition — would make the export path the cheap way around the
    visibility gate, and a file is the one response a caller walks away with.
    Identical means identical: status, problem class, reason and context are
    compared field-for-field, and the body must parse as a bare problem
    document — zero evidence bytes, no file offered.
    """
    outsider = stand_manifest.fixture("sales_ic")
    request = _seeded_request(stand_manifest, entity_id=outsider.uuid)

    paged = api.post(DRILLDOWN, json_body=request)
    paged_document = _refusal(paged, 403)

    for file_format in ("csv", "xlsx"):
        response = _export(api, request, file_format)
        document = _refusal(response, 403)
        assert response.content_type.startswith(PROBLEM_CONTENT_TYPE), (
            f"{file_format}: a refusal served as {response.content_type!r}, not a problem document"
        )
        assert response.headers.get("content-disposition") is None, (
            f"{file_format}: a refusal must not offer a file, got "
            f"{response.headers.get('content-disposition')!r}"
        )
        assert (document.type, document.title, document.detail, document.context) == (
            paged_document.type,
            paged_document.title,
            paged_document.detail,
            paged_document.context,
        ), f"{file_format}: the export's refusal differs from the paged read's"
