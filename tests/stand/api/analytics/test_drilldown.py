"""`/v1/metric-drilldown` and its export — the evidence behind a metric value.

    POST /v1/metric-drilldown         200 · 400 empty-entity · 403 hidden person
    POST /v1/metric-drilldown/export  200 CSV/XLSX · 400 empty-entity

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
them, and the metrics not in it are deliberately not exported here.
"""

from __future__ import annotations

import csv
import io
import math
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
    MetricDefinitionListResponse,
    MetricResultsResponse,
    PeriodView,
    ProblemDocument,
)
from ..schemas.analytics import (
    MetricDrilldownCapability,
    MetricDrilldownResponse,
)
from .drilldown_matrix import EXPORT_SHAPES, MATRIX, Expectation, Tier

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

#: Aggregation order differs between the service (vectorized `sumIf` over
#: `Float64`) and this suite (row-ordered `sum`), and that is the only error
#: source — the transport is lossless both ways.
_REL_TOL = 1e-9
_ABS_TOL = 1e-9

#: Well-formed apart from the one field under test. An empty entity id is the
#: cheapest rejection that is unambiguously the HANDLER's — it needs no seeded
#: metric to reach, and no lookup can turn it into a 404 on the way.
_EMPTY_ENTITY_ID = ""


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


def _request_for(
    manifest: Manifest,
    metric_key: str,
    *,
    entity_id: str | None = None,
    limit: int | None = None,
    cursor: str | None = None,
    filters: Sequence[JsonValue] = (),
    display_dimensions: Sequence[str] = (),
) -> dict[str, JsonValue]:
    start, _, end = manifest.data_window.partition("..")
    request: dict[str, JsonValue] = {
        "metric_key": metric_key,
        "entity": {
            "type": "person",
            # Not `or`: an empty id is a case a caller may want to send, and
            # falling back to a real person would quietly test something else.
            "id": manifest.fixture("dev_lead").uuid if entity_id is None else entity_id,
        },
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
    cursor: str | None = None,
    filters: Sequence[JsonValue] = (),
    display_dimensions: Sequence[str] = (),
) -> ApiResponse:
    return api.post(
        DRILLDOWN,
        json_body=_request_for(
            manifest,
            metric_key,
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
    filters: Sequence[JsonValue] = (),
) -> float | None:
    """The scalar the dashboard shows for the same person, period and filters.

    `None` is a real answer rather than a failure: nothing zero-fills, so a
    person with no observations in the period has no value at all, and that is
    what an empty evidence page has to agree with.
    """
    start, _, end = manifest.data_window.partition("..")
    person_id = manifest.fixture("dev_lead").uuid
    response = api.post(
        METRIC_RESULTS,
        json_body={
            "entity": {"type": "person", "ids": [person_id]},
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
    assert values[0].entity_id == person_id
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


def _assert_shape(walk: _Walk, expectation: Expectation, person_id: str) -> None:
    selection = walk.first.selection
    assert selection.metric_key == expectation.metric_key
    assert selection.entity.id == person_id
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
    "expectation", MATRIX, ids=[expectation.metric_key for expectation in MATRIX]
)
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
    probe = _page(api, stand_manifest, expectation.metric_key, limit=_PAGE_LIMIT)
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
        page_budget=_PAGE_BUDGET,
        initial=probe,
    )
    _assert_shape(walk, expectation, person_id)

    period = _period_value(api, stand_manifest, expectation.metric_key)
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
def test_git_commit_drilldown_pages_and_reconciles(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    walk = _walk(
        api,
        stand_manifest,
        GIT_COMMITS,
        limit=1,
        filters=[{"dimension": "source", "values": ["github"]}],
        display_dimensions=["repository"],
    )
    person_id = stand_manifest.fixture("dev_lead").uuid

    assert walk.first.selection.metric_key == GIT_COMMITS
    assert walk.first.selection.entity.id == person_id
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
def test_git_commit_drilldown_exports_all_rows(api: ApiClient, stand_manifest: Manifest) -> None:
    walk = _walk(
        api,
        stand_manifest,
        GIT_COMMITS,
        limit=1,
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
