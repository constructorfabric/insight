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
    MetricDefinitionListResponse,
    MetricResultsResponse,
    PeriodView,
    ProblemDocument,
)

METRIC_RESULTS = analytics_path("/v1/metric-results")


def _a_metric_key(api: ApiClient) -> str:
    response = api.get(analytics_path("/v1/metric-definitions"))
    assert response.status_code == 200, f"definitions: {response.status_code}"
    metrics = response.parse(MetricDefinitionListResponse).metrics
    assert metrics, "no metric definitions — did the migrations run?"
    return metrics[0].metric_key


def _ask(api: ApiClient, manifest: Manifest, entity_id: str, metric_key: str) -> ApiResponse:
    start, _, end = manifest.data_window.partition("..")
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


def test_metric_results_200(api: ApiClient, stand_manifest: Manifest) -> None:
    """One person, the seeded window, one metric — and a REAL number back.

    The entity id is the person's canonical UUID — since the identity cutover
    (#2098) that is what every person-keyed route takes, and an email is
    refused rather than answered emptily (below).

    The period comes from the manifest's own `data_window`, so the request asks
    for the range the stand was actually seeded over rather than a guess.

    Asserting a non-null value is the whole point. A 200 whose values are all
    null is what this test used to accept, and it proved only that the route
    was reachable — the chain from gold through the tenant in the JWT to a
    number on the wire was never exercised.
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
    start, _, end = manifest.data_window.partition("..")
    return {
        "entity": {"type": "person", "ids": [manifest.fixture("dev_lead").uuid]},
        "period": {"from": start, "to": end},
        "metrics": [{"metric_key": _a_metric_key(api), "views": [{"view": "period"}]}],
    }


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
