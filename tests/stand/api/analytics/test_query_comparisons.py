"""`POST /v1/query/comparisons` — a person's value beside the spread around it.

    POST /v1/query/comparisons  200 · 400 a window that runs backwards
                                403 outside the visible set · 404 unknown metric

The 401 half is in `test_gateway.py` and the 415 half in
`test_request_contracts.py`, both swept over every operation at once.

Nothing here asserts a population number. What the answer must satisfy is
internal: a size that is reported whatever it is, readings that are either
withheld or ordered, and no member of the population named anywhere — the
disclosure invariant the endpoint exists to keep.
"""

from __future__ import annotations

import pytest
from insight_stand import ApiClient, Manifest, analytics_path
from insight_stand.api import JsonValue

from ..schemas import ProblemDocument
from ..schemas.analytics import ComparisonsResponse, PopulationSpread
from . import query_window

QUERY_COMPARISONS = analytics_path("/v1/query/comparisons")

#: Declares a cohort in the shipped definitions, so `population: cohort` has one
#: to compare within.
GIT_COMMITS = "git.commits"

#: Well-formed and carried by no definition, so a refusal is the catalogue's and
#: not a spelling rejection dressed as one.
UNKNOWN_METRIC = "stand.does_not_exist"


def _question(manifest: Manifest, metric: str, target_id: str) -> dict[str, JsonValue]:
    start, end = query_window(manifest)
    return {
        "metric": metric,
        "targets": [target_id],
        "population": {"type": "cohort"},
        "time": {"from": start, "to": end},
    }


def _readings(spread: PopulationSpread) -> list[float | None]:
    """The spread's positions in the order a distribution puts them."""
    return [spread.min, spread.p25, spread.median, spread.p75, spread.max]


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_query_comparisons_reports_a_population_that_reads_as_one_spread(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    """A size is always reported, and whatever readings come with it are ordered."""
    person = stand_manifest.fixture("dev_lead")

    response = api.post(
        QUERY_COMPARISONS,
        json_body={"queries": [_question(stand_manifest, GIT_COMMITS, person.uuid)]},
    )
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    results = response.parse(ComparisonsResponse).results
    assert [result.metric for result in results] == [GIT_COMMITS]
    assert [target.subject for target in results[0].targets] == [person.uuid]

    spread = results[0].targets[0].population
    assert spread.n >= 0

    readings = _readings(spread)
    if spread.n == 0:
        assert readings == [None] * len(readings), (
            f"nothing was observed, yet the spread reports {readings}"
        )

    reported = [reading for reading in readings if reading is not None]
    assert reported == sorted(reported), f"the spread is not ordered: {readings}"


@pytest.mark.requires_seed("sales_ic")
@pytest.mark.security
def test_query_comparisons_refuses_a_target_out_of_scope(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    """A comparison naming somebody outside the lead's subtree is refused, not answered."""
    outsider = stand_manifest.fixture("sales_ic")

    response = api.post(
        QUERY_COMPARISONS,
        json_body={"queries": [_question(stand_manifest, GIT_COMMITS, outsider.uuid)]},
    )
    assert response.status_code == 403, (
        f"comparing {outsider.email}, who is outside the lead's scope, answered "
        f"{response.status_code}: {response.text[:300]}"
    )
    assert response.parse(ProblemDocument).status == 403


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_query_comparisons_reports_an_unknown_metric_as_not_found(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    """A key the definitions do not carry is refused before any population is read."""
    person = stand_manifest.fixture("dev_lead")

    response = api.post(
        QUERY_COMPARISONS,
        json_body={"queries": [_question(stand_manifest, UNKNOWN_METRIC, person.uuid)]},
    )
    assert response.status_code == 404, (
        f"an unknown metric answered {response.status_code}, expected 404: {response.text[:300]}"
    )
    assert response.parse(ProblemDocument).status == 404


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_query_comparisons_refuses_a_window_that_runs_backwards(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    """A range that parses but cannot be honoured is refused, not answered emptily."""
    person = stand_manifest.fixture("dev_lead")
    question = _question(stand_manifest, GIT_COMMITS, person.uuid)
    question["time"] = {"from": "2026-02-01", "to": "2026-01-01"}

    response = api.post(QUERY_COMPARISONS, json_body={"queries": [question]})
    assert response.status_code == 400, f"status={response.status_code} {response.text[:300]}"
    assert response.parse(ProblemDocument).status == 400
