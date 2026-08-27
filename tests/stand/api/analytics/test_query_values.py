"""`POST /v1/query/values` — the value questions the semantic definitions answer.

    POST /v1/query/values   200 · 400 a request asking nothing
                            403 outside the visible set · 404 unknown metric

The 401 half is in `test_gateway.py` and the 415 half in
`test_request_contracts.py`, both swept over every operation at once.

No number is written into this file. The 200 case asks one question twice in one
request — once folded over the whole window, once cut into days — and requires
the two answers to agree, so the seed can change underneath it and a
disagreement is a defect rather than a stale expectation.
"""

from __future__ import annotations

import math

import pytest
from insight_stand import ApiClient, Manifest, analytics_path
from insight_stand.api import JsonValue

from ..schemas import ProblemDocument
from ..schemas.analytics import ResultBody1, ResultBody2, ValuesResponse
from . import query_window

QUERY_VALUES = analytics_path("/v1/query/values")

#: Counted per commit by the shipped definitions, so a window total and the sum
#: of that window's daily points count the same rows.
GIT_COMMITS = "git.commits"

#: Well-formed and carried by no definition, so a refusal is the catalogue's and
#: not a spelling rejection dressed as one.
UNKNOWN_METRIC = "stand.does_not_exist"

#: The service folds a window with a vectorized sum and this suite adds points
#: in row order, so the two agree to floating-point accumulation and no further.
_REL_TOL = 1e-9
_ABS_TOL = 1e-9


def _question(manifest: Manifest, metric: str, subject_id: str, grain: str) -> dict[str, JsonValue]:
    start, end = query_window(manifest)
    return {
        "metric": metric,
        "subjects": {"type": "persons", "ids": [subject_id]},
        "time": {"from": start, "to": end, "grain": grain},
        "fold": "per_subject",
    }


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_query_values_answers_a_window_folded_and_cut_to_the_same_number(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    """The window total, the series total and the sum of the daily points all agree."""
    person = stand_manifest.fixture("dev_lead")

    response = api.post(
        QUERY_VALUES,
        json_body={
            "queries": [
                _question(stand_manifest, GIT_COMMITS, person.uuid, "total"),
                _question(stand_manifest, GIT_COMMITS, person.uuid, "day"),
            ]
        },
    )
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    results = response.parse(ValuesResponse).results
    assert [result.metric for result in results] == [GIT_COMMITS, GIT_COMMITS]

    folded = results[0].result.root
    cut = results[1].result.root
    assert isinstance(folded, ResultBody1), f"a total grain answered {type(folded).__name__}"
    assert isinstance(cut, ResultBody2), f"a day grain answered {type(cut).__name__}"

    assert [value.subject for value in folded.values] == [person.uuid]
    total = folded.values[0].value
    assert total is not None, (
        f"{GIT_COMMITS} came back null over the seeded window {stand_manifest.data_window} — "
        f"the request reached the service but no data answered it"
    )

    assert [series.subject for series in cut.series] == [person.uuid]
    series = cut.series[0]
    assert series.total is not None and math.isclose(
        series.total, total, rel_tol=_REL_TOL, abs_tol=_ABS_TOL
    ), f"the series reports {series.total} for the window the total grain answered {total} for"

    summed = sum(point.value for point in series.points if point.value is not None)
    assert math.isclose(summed, total, rel_tol=_REL_TOL, abs_tol=_ABS_TOL), (
        f"the daily points add up to {summed}, and the same window folded once is {total}"
    )


@pytest.mark.requires_seed("sales_ic")
@pytest.mark.security
def test_query_values_refuses_a_person_out_of_scope(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    """A question about somebody outside the lead's subtree is refused, not answered emptily."""
    outsider = stand_manifest.fixture("sales_ic")

    response = api.post(
        QUERY_VALUES,
        json_body={"queries": [_question(stand_manifest, GIT_COMMITS, outsider.uuid, "total")]},
    )
    assert response.status_code == 403, (
        f"asking about {outsider.email}, who is outside the lead's scope, answered "
        f"{response.status_code}: {response.text[:300]}"
    )
    assert response.parse(ProblemDocument).status == 403


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_query_values_reports_an_unknown_metric_as_not_found(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    """A key the definitions do not carry is 404 here, where `/v1/metric-results` says 400."""
    person = stand_manifest.fixture("dev_lead")

    response = api.post(
        QUERY_VALUES,
        json_body={"queries": [_question(stand_manifest, UNKNOWN_METRIC, person.uuid, "total")]},
    )
    assert response.status_code == 404, (
        f"an unknown metric answered {response.status_code}, expected 404: {response.text[:300]}"
    )
    assert response.parse(ProblemDocument).status == 404


@pytest.mark.reliability
def test_query_values_refuses_a_request_that_asks_nothing(api: ApiClient) -> None:
    """A request carrying no question is malformed, not an empty answer."""
    response = api.post(QUERY_VALUES, json_body={"queries": []})
    assert response.status_code == 400, f"status={response.status_code} {response.text[:300]}"
    assert response.parse(ProblemDocument).status == 400
