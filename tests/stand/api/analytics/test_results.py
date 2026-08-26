"""`POST /v1/metric-results` — the endpoint the dashboard actually calls.

    POST /v1/metric-results   200 · 400 (empty metrics, bad period, unknown key,
                                        a key that is not a person id)
                              403 outside the visible set · 422 off-schema

The widest single request in the API: an entity, a period and a list of metrics
each with its own views. Worth a deployed-path test more than most, because it
is the one request whose failure a user would see directly, and because its
result depends on the whole chain — the session's tenant reaching ClickHouse,
gold views having been built, and the seeded window overlapping the period asked
for.

Asserted against the metric DEFINITIONS rather than a hardcoded key, so a
catalogue change moves the test without editing it.

The 401 half is in `test_gateway.py`, swept over every operation at once.
"""

from __future__ import annotations

import pytest
from insight_stand import ApiClient, ApiResponse, Manifest, analytics_path
from insight_stand.api import JsonValue

from ..schemas import (
    BreakdownView,
    MetricDefinitionListResponse,
    MetricResultsResponse,
    PeriodView,
    ProblemDocument,
    RollupView,
)
from . import query_window

METRIC_RESULTS = analytics_path("/v1/metric-results")


def _a_metric_key(api: ApiClient) -> str:
    response = api.get(analytics_path("/v1/metric-definitions"))
    assert response.status_code == 200, f"definitions: {response.status_code}"
    metrics = response.parse(MetricDefinitionListResponse).metrics
    assert metrics, "no metric definitions — did the migrations run?"
    return metrics[0].metric_key


def _ask(api: ApiClient, manifest: Manifest, entity_id: str, metric_key: str) -> ApiResponse:
    start, end = query_window(manifest)
    return api.post(
        METRIC_RESULTS,
        json_body={
            "entity": {"type": "person", "ids": [entity_id]},
            "period": {"from": start, "to": end},
            "metrics": [{"metric_key": metric_key, "views": [{"view": "period"}]}],
        },
    )


def _values(response: ApiResponse, metric_key: str) -> list[tuple[str, float | None]]:
    """Every (entity_id, value) pair the response carries for one metric.

    `MetricResultDto` is a RootModel over four view-shaped variants, so the
    payload unwraps once. That the union resolved at all is part of the
    assertion: the response matched a shape the contract declares, rather than
    something the models had to be loosened to accept.
    """
    results = response.parse(MetricResultsResponse)
    answered = [metric.root.metric_key for metric in results.metrics]
    assert answered == [metric_key], (
        f"asked for {metric_key!r} and the response answered for {answered}"
    )

    pairs: list[tuple[str, float | None]] = []
    for metric in results.metrics:
        for view in metric.root.views:
            # The view is a union of five shapes and only the period one carries
            # `values`. Narrowing it IS an assertion rather than a workaround:
            # the request asked for `{"view": "period"}`, so a different variant
            # coming back means the service answered a question nobody asked.
            assert isinstance(view.root, PeriodView), (
                f"asked for the period view and got {type(view.root).__name__}"
            )
            pairs += [(value.entity_id, value.value) for value in view.root.values]
    return pairs


@pytest.mark.reliability
@pytest.mark.stand_smoke
def test_metric_results_200(api: ApiClient, stand_manifest: Manifest) -> None:
    """One person, the seeded window, one metric — and a REAL number back.

    The entity id is the person's canonical UUID — since the identity cutover
    (#2098) that is what every person-keyed route takes, and an email is
    refused rather than answered emptily (below).

    The period is the queryable TAIL of the manifest's own `data_window` (see
    `query_window`), so the request asks for the most recent slice of the range
    the stand was actually seeded over, capped at what the API will accept,
    rather than a guess.

    Asserting a non-null value is the whole point. A 200 whose values are all
    null is what this test used to accept, and it proved only that the route
    was reachable — the chain from gold through the tenant in the JWT to a
    number on the wire was never exercised.

    Marked `stand_smoke`: this is the post-deploy gate's "the seeded data
    reaches the API" check, folded into the suite that already runs it on
    every other lane rather than duplicated in a package of its own.
    """
    person = stand_manifest.fixture("dev_lead")
    metric_key = _a_metric_key(api)

    response = _ask(api, stand_manifest, person.uuid, metric_key)
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    values = _values(response, metric_key)
    assert values, f"the response carried no values at all: {response.text[:300]}"
    assert [entity for entity, _ in values] == [person.uuid]
    assert all(value is not None for _, value in values), (
        f"{metric_key} came back null for {person.email} over the seeded window "
        f"{stand_manifest.data_window} — the request reached the service but no "
        f"gold data answered it: {values}"
    )


@pytest.mark.security
def test_metric_results_403_for_a_person_out_of_scope(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    """The visibility gate, reached with a well-formed key.

        The uuid is the right key since the identity cutover, so the refusal has to
    come from the person rather than from the spelling — `sales_ic` is outside
    a development lead's subtree, the same pair `test_subchart.py` uses for its
    out-of-scope 404.

    Still the only case in this module that reaches the gate: the request is
    the 200's, differing solely in whose id it names.
    """
    outsider = stand_manifest.fixture("sales_ic")
    metric_key = _a_metric_key(api)

    response = _ask(api, stand_manifest, outsider.uuid, metric_key)
    assert response.status_code == 403, (
        f"asking about {outsider.email}, who is outside the lead's scope, answered "
        f"{response.status_code}: {response.text[:300]}"
    )
    assert response.parse(ProblemDocument).status == 403


@pytest.mark.parametrize(
    ("label", "entity_id"),
    [
        ("pre-cutover email", "somebody@example.com"),
        ("nil uuid", "00000000-0000-0000-0000-000000000000"),
    ],
)
@pytest.mark.reliability
def test_metric_results_400_for_a_key_that_is_not_a_person_id(
    api: ApiClient, stand_manifest: Manifest, label: str, entity_id: str
) -> None:
    """Never a silent empty result.

    An email is what this endpoint took before the cutover, so an unmigrated
    caller sends one in earnest — and a 200 with `value: null` would be
    indistinguishable from a person who genuinely has no activity. That
    ambiguity is the exact failure the cutover removed; this keeps it removed.
    """
    response = _ask(api, stand_manifest, entity_id, _a_metric_key(api))
    assert response.status_code == 400, (
        f"a {label} answered {response.status_code} rather than 400: {response.text[:300]}"
    )


@pytest.mark.reliability
def test_metric_results_422_off_schema(api: ApiClient) -> None:
    """A body that is valid JSON but not the request type.

    Axum's own extractor rejection, so it arrives as `text/plain` rather than a
    canonical problem document — see `schemas/common.EXTRACTOR_REJECTION_*`
    and #1670. Asserted as it behaves.
    """
    response = api.post(METRIC_RESULTS, json_body={"not": "a metric-results request"})
    assert response.status_code == 422, f"status={response.status_code} {response.text[:300]}"


# ---------------------------------------------------------------------------
# Request validation, all of it before ClickHouse is touched
# ---------------------------------------------------------------------------


def _body(api: ApiClient, manifest: Manifest) -> dict[str, JsonValue]:
    start, end = query_window(manifest)
    return {
        "entity": {"type": "person", "ids": [manifest.fixture("dev_lead").uuid]},
        "period": {"from": start, "to": end},
        "metrics": [{"metric_key": _a_metric_key(api), "views": [{"view": "period"}]}],
    }


@pytest.mark.reliability
def test_an_empty_metrics_list_is_400(api: ApiClient, stand_manifest: Manifest) -> None:
    """Nothing asked for is a malformed request, not an empty answer."""
    body = _body(api, stand_manifest)
    body["metrics"] = []

    response = api.post(METRIC_RESULTS, json_body=body)
    assert response.status_code == 400, f"status={response.status_code} {response.text[:300]}"


@pytest.mark.parametrize(
    ("label", "period"),
    [
        ("unparseable", {"from": "not-a-date", "to": "2026-01-31"}),
        ("reversed", {"from": "2026-02-01", "to": "2026-01-01"}),
    ],
)
@pytest.mark.reliability
def test_a_period_that_cannot_be_honoured_is_400(
    api: ApiClient, stand_manifest: Manifest, label: str, period: dict[str, str]
) -> None:
    """Both rejected up front, before any bucket enumeration.

    The reversed case is the one worth having: it parses, so nothing forces the
    service to notice, and an unnoticed reversed range answers 200 with no rows
    — again indistinguishable from no data.
    """
    body = _body(api, stand_manifest)
    body["period"] = dict(period)

    response = api.post(METRIC_RESULTS, json_body=body)
    assert response.status_code == 400, (
        f"a {label} period answered {response.status_code}: {response.text[:300]}"
    )


@pytest.mark.reliability
def test_an_unknown_metric_key_is_400_not_404(api: ApiClient, stand_manifest: Manifest) -> None:
    """This endpoint has no not-found path, and the spec declares none.

    A metric_key that resolves to nothing is `unavailable` — a statement about
    the request, not about a missing resource. Pinning 400 specifically is what
    keeps the contract and the handler from drifting apart, since 404 is the
    intuitive answer and the wrong one.
    """
    body = _body(api, stand_manifest)
    body["metrics"] = [
        {"metric_key": "stand.definitely-not-a-real-metric", "views": [{"view": "period"}]}
    ]

    response = api.post(METRIC_RESULTS, json_body=body)
    assert response.status_code == 400, (
        f"an unknown metric_key answered {response.status_code}, expected 400 unavailable: "
        f"{response.text[:300]}"
    )


@pytest.mark.requires_seed("dev_lead", "sales_ic")
@pytest.mark.security
def test_one_hidden_person_refuses_the_whole_request(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    """Mixing a visible person with a hidden one is refused, not filtered.

    The alternative — dropping the unauthorized entity and answering 200 for
    the rest — is worse than it looks: the caller cannot tell a person they may
    not see from a person with no activity, so a dashboard would render a
    confident zero. Refusing the request keeps the distinction observable.

    `sales_ic` is outside a development lead's subchart, the same pair
    `test_subchart.py` uses for its out-of-scope 404.
    """
    body = _body(api, stand_manifest)
    body["entity"] = {
        "type": "person",
        "ids": [
            stand_manifest.fixture("dev_lead").uuid,
            stand_manifest.fixture("sales_ic").uuid,
        ],
    }

    response = api.post(METRIC_RESULTS, json_body=body)
    assert response.status_code == 403, (
        f"a request naming one hidden person answered {response.status_code} — if it "
        f"succeeded, the hidden entity was silently dropped: {response.text[:300]}"
    )


def _git_definitions(api: ApiClient) -> dict[str, list[str]]:
    """Every git metric the installation offers, keyed by metric_key → dimensions.

    The listing carries no `computation`, so what a rollup can be asserted
    against is read from the dimension list instead — which is the property
    that actually decides whether a metric can be grouped by repository. The
    keys this file cares about are recent enough that a stand pinned to an
    older image simply will not have them, so the catalogue decides rather
    than a hardcoded list.
    """
    response = api.get(analytics_path("/v1/metric-definitions"))
    assert response.status_code == 200, f"definitions: {response.status_code}"
    return {
        metric.metric_key: list(metric.dimensions)
        for metric in response.parse(MetricDefinitionListResponse).metrics
        if metric.metric_key.startswith("git.")
    }


#: Summed git metrics, most preferred first. Every one of them counts whole
#: records, so a rollup's rows add up to the same period total — which is what
#: the reconciliation below asserts. Hardcoded because the listing does not say
#: which computation a metric uses, and a median would need a different oracle.
_SUMMED_GIT_METRICS = ("git.prs_merged", "git.commits", "git.code_lines")


def _a_summed_git_metric(api: ApiClient) -> str | None:
    """A summed git metric this install offers that groups by repository."""
    dimensions = _git_definitions(api)
    return next(
        (key for key in _SUMMED_GIT_METRICS if "repository" in dimensions.get(key, [])),
        None,
    )


def _rollup_view(response: ApiResponse, metric_key: str) -> RollupView:
    """The one rollup view the response carries for a metric.

    Narrowing the union IS the assertion, as in `_values`: the request asked
    for a rollup, so any other variant means the service answered a question
    nobody asked.
    """
    results = response.parse(MetricResultsResponse)
    views = [view.root for metric in results.metrics for view in metric.root.views]
    assert len(views) == 1, f"asked for one view of {metric_key} and got {len(views)}"
    view = views[0]
    assert isinstance(view, RollupView), f"asked for the rollup view and got {type(view).__name__}"
    return view


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_rollup_answers_per_dimension_without_an_entity_grain(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    """A repository rollup: rows keyed by the dimension, never by a person.

    The shape is the whole point. `rollup` is the one view whose rows carry no
    `entity_id` — the Repositories lens reads it precisely because a
    per-repository number cannot be assembled from per-person rows without
    knowing which people to add. A row that grew an entity id back would break
    that screen while still answering 200.

    `contributing_entity_count` is asserted against the roster it was asked
    over rather than a number: it counts DISTINCT resolved persons among the
    entities in the request, so it can never exceed how many were named, and a
    value above that would mean the count is measuring something else.

    Skipped rather than failed when the installation offers no git metric with
    a `repository` dimension — a stand seeded without a git connector has
    nothing to roll up, which is not this test's news to report.
    """
    metric_key = _a_summed_git_metric(api)
    if metric_key is None:
        pytest.skip("this installation offers no summed git metric to roll up")

    person = stand_manifest.fixture("dev_lead")
    start, end = query_window(stand_manifest)
    response = api.post(
        METRIC_RESULTS,
        json_body={
            "entity": {"type": "person", "ids": [person.uuid]},
            "period": {"from": start, "to": end},
            "metrics": [
                {
                    "metric_key": metric_key,
                    "views": [{"view": "rollup", "dimensions": ["repository"]}],
                }
            ],
        },
    )
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    view = _rollup_view(response, metric_key)
    assert view.dimensions == ["repository"]
    for value in view.values:
        keys = [dimension.key for dimension in value.dimensions]
        # A remainder row names no dimension value; this request set no group
        # limit, so every row here is a real repository.
        assert keys == ["repository"], f"a rollup row named {keys}"
        assert value.contributing_entity_count <= 1, (
            f"one person was asked about and the row counted "
            f"{value.contributing_entity_count} contributors: {value}"
        )


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_a_rollup_totals_the_same_work_the_period_view_reports(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    """Two independent reads of one number have to agree.

    The oracle is the API's own period view rather than an expectation written
    here: for a summed metric, every repository's share added together IS the
    person's total over the same window. The two answers travel different SQL
    (a grouped aggregate against an ungrouped one), so a drift between them is
    a real defect in one of the two paths — and neither number is hand-authored,
    which keeps the case honest on any seed.

    A rollup that returns nothing is a skip, not a pass: a metric the stand has
    no git rows for would otherwise let 0 == 0 through as agreement.
    """
    metric_key = _a_summed_git_metric(api)
    if metric_key is None:
        pytest.skip("this installation offers no summed git metric to roll up")

    person = stand_manifest.fixture("dev_lead")
    start, end = query_window(stand_manifest)
    body: dict[str, JsonValue] = {
        "entity": {"type": "person", "ids": [person.uuid]},
        "period": {"from": start, "to": end},
        "metrics": [
            {
                "metric_key": metric_key,
                "views": [{"view": "rollup", "dimensions": ["repository"]}],
            }
        ],
    }
    rolled = api.post(METRIC_RESULTS, json_body=body)
    assert rolled.status_code == 200, f"status={rolled.status_code} {rolled.text[:300]}"
    rows = _rollup_view(rolled, metric_key).values
    if not rows:
        pytest.skip(f"{metric_key} has no rows on this stand to reconcile")

    period = _ask(api, stand_manifest, person.uuid, metric_key)
    assert period.status_code == 200, f"status={period.status_code} {period.text[:300]}"
    total = sum(value or 0 for _, value in _values(period, metric_key))
    grouped = sum(row.value or 0 for row in rows)

    assert grouped == pytest.approx(total), (
        f"{metric_key} over {start}..{end}: the repositories add up to {grouped} "
        f"while the period view reports {total} — the grouped and ungrouped "
        f"paths disagree about the same work"
    )


@pytest.mark.requires_seed("dev_lead", "sales_ic")
@pytest.mark.security
def test_a_rollup_refuses_a_person_out_of_scope(api: ApiClient, stand_manifest: Manifest) -> None:
    """The visibility gate holds on the view with no entity grain.

    Worth its own case precisely BECAUSE the answer carries no entity ids: a
    rollup that skipped the gate would leak an outsider's work as an
    unattributed total, which no reader could trace back to a person and no
    other assertion in this module would catch.
    """
    metric_key = next(iter(_git_definitions(api)), None)
    if metric_key is None:
        pytest.skip("this installation offers no git metric")

    start, end = query_window(stand_manifest)
    response = api.post(
        METRIC_RESULTS,
        json_body={
            "entity": {
                "type": "person",
                "ids": [stand_manifest.fixture("sales_ic").uuid],
            },
            "period": {"from": start, "to": end},
            "metrics": [
                {
                    "metric_key": metric_key,
                    "views": [{"view": "rollup", "dimensions": ["repository"]}],
                }
            ],
        },
    )
    assert response.status_code == 403, (
        f"a rollup over a person outside the caller's scope answered "
        f"{response.status_code}: {response.text[:300]}"
    )


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.versatility
def test_pr_comments_split_into_own_and_others(api: ApiClient, stand_manifest: Manifest) -> None:
    """`comment_target` is a two-valued dimension, and only those two values.

    The split is what makes the metric readable — reviewing someone else's
    request is different work from answering on your own — so a third value
    arriving (an empty string, an `__unknown__` placeholder leaking from gold)
    would silently turn one of the two columns into a third.

    Skipped where the installation predates the metric: a stand pinned to an
    older analytics image has no such key, which the catalogue answers for.
    """
    if "git.pr_comments" not in _git_definitions(api):
        pytest.skip("this installation does not offer git.pr_comments")

    person = stand_manifest.fixture("dev_lead")
    start, end = query_window(stand_manifest)
    response = api.post(
        METRIC_RESULTS,
        json_body={
            "entity": {"type": "person", "ids": [person.uuid]},
            "period": {"from": start, "to": end},
            "metrics": [
                {
                    "metric_key": "git.pr_comments",
                    "views": [{"view": "breakdown", "dimensions": ["comment_target"]}],
                }
            ],
        },
    )
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    results = response.parse(MetricResultsResponse)
    views = [view.root for metric in results.metrics for view in metric.root.views]
    assert len(views) == 1 and isinstance(views[0], BreakdownView), (
        f"asked for the breakdown view and got {[type(v).__name__ for v in views]}"
    )
    seen = {
        dimension.value
        for value in views[0].values
        for dimension in value.dimensions
        if dimension.key == "comment_target"
    }
    assert seen <= {"own", "others"}, (
        f"comment_target answered with {sorted(seen)} — own/others is the whole "
        f"vocabulary, so a third value means gold classified something it could "
        f"not name"
    )
