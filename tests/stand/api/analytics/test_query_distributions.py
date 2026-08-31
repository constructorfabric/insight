"""`POST /v1/query/distributions` — the shape of a subject's own per-row values.

    POST /v1/query/distributions  200 · 400 a quantile outside (0, 1)
                                  403 outside the visible set · 404 unknown metric

The 401 half is in `test_gateway.py` and the 415 half in
`test_request_contracts.py`, both swept over every operation at once.

`git.pr_size` rather than `git.commits`: only a metric whose computation is
taken over its measure's per-row values has a distribution at all, and the
shipped percentile metrics are the ones that do.
"""

from __future__ import annotations

import math

import pytest
from insight_stand import ApiClient, Manifest, analytics_path
from insight_stand.api import JsonValue

from ..schemas import ProblemDocument
from ..schemas.analytics import DistributionsResponse, Histogram
from . import query_window

QUERY_DISTRIBUTIONS = analytics_path("/v1/query/distributions")

#: A percentile computation over per-PR change size, so it has a distribution.
GIT_PR_SIZE = "git.pr_size"

#: Well-formed and carried by no definition, so a refusal is the catalogue's and
#: not a spelling rejection dressed as one.
UNKNOWN_METRIC = "stand.does_not_exist"

#: A percentile computation over per-run duration, keyed by the tenant rather
#: than by a person.
CI_RUN_DURATION = "ci.run_duration_min"

#: Asked sorted and distinct, which is how the service reports them back.
_QUANTILES: tuple[float, ...] = (0.25, 0.5, 0.75)

_BINS = 10

#: Bin edges are derived from the same two bounds by the same arithmetic, so
#: neighbouring edges agree to floating-point accumulation and no further.
_REL_TOL = 1e-9
_ABS_TOL = 1e-9


def _question(manifest: Manifest, metric: str, subject_id: str) -> dict[str, JsonValue]:
    start, end = query_window(manifest)
    return {
        "metric": metric,
        "subjects": {"type": "persons", "ids": [subject_id]},
        "time": {"from": start, "to": end},
        "bins": _BINS,
        "quantiles": list(_QUANTILES),
    }


def _assert_bins_tile_the_range(histogram: Histogram) -> None:
    if not histogram.bins:
        assert (histogram.lo, histogram.hi) == (None, None), (
            f"no bins were cut, yet the range is {histogram.lo}..{histogram.hi}"
        )
        return

    assert histogram.lo is not None and histogram.hi is not None
    assert math.isclose(histogram.bins[0].lo, histogram.lo, rel_tol=_REL_TOL, abs_tol=_ABS_TOL)
    assert math.isclose(histogram.bins[-1].hi, histogram.hi, rel_tol=_REL_TOL, abs_tol=_ABS_TOL)

    for lower, upper in zip(histogram.bins, histogram.bins[1:], strict=False):
        assert lower.lo <= lower.hi, f"a bin runs backwards: {lower.lo}..{lower.hi}"
        assert math.isclose(lower.hi, upper.lo, rel_tol=_REL_TOL, abs_tol=_ABS_TOL), (
            f"the bins leave a gap: {lower.hi} then {upper.lo}"
        )


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_query_distributions_cuts_a_range_its_own_readings_agree_with(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    """The bins tile the reported range once, and the quantiles rise with their positions."""
    person = stand_manifest.fixture("dev_lead")

    response = api.post(
        QUERY_DISTRIBUTIONS,
        json_body={"queries": [_question(stand_manifest, GIT_PR_SIZE, person.uuid)]},
    )
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    results = response.parse(DistributionsResponse).results
    assert [result.metric for result in results] == [GIT_PR_SIZE]
    assert [subject.subject for subject in results[0].subjects] == [person.uuid]

    answered = results[0].subjects[0]
    assert answered.histogram is not None, "a question naming bins was answered without a histogram"
    assert len(answered.histogram.bins) <= _BINS
    _assert_bins_tile_the_range(answered.histogram)

    assert answered.quantiles is not None, "a question naming quantiles was answered without any"
    assert [quantile.q for quantile in answered.quantiles] == list(_QUANTILES)
    reported = [quantile.value for quantile in answered.quantiles if quantile.value is not None]
    assert reported == sorted(reported), (
        f"the quantiles do not rise with their positions: {reported}"
    )


@pytest.mark.requires_seed("sales_ic")
@pytest.mark.security
def test_query_distributions_refuses_a_person_out_of_scope(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    """A distribution of somebody outside the lead's subtree is refused, not answered."""
    outsider = stand_manifest.fixture("sales_ic")

    response = api.post(
        QUERY_DISTRIBUTIONS,
        json_body={"queries": [_question(stand_manifest, GIT_PR_SIZE, outsider.uuid)]},
    )
    assert response.status_code == 403, (
        f"asking about {outsider.email}, who is outside the lead's scope, answered "
        f"{response.status_code}: {response.text[:300]}"
    )
    assert response.parse(ProblemDocument).status == 403


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_query_distributions_reports_an_unknown_metric_as_not_found(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    """A key the definitions do not carry is refused before the distribution check runs."""
    person = stand_manifest.fixture("dev_lead")

    response = api.post(
        QUERY_DISTRIBUTIONS,
        json_body={"queries": [_question(stand_manifest, UNKNOWN_METRIC, person.uuid)]},
    )
    assert response.status_code == 404, (
        f"an unknown metric answered {response.status_code}, expected 404: {response.text[:300]}"
    )
    assert response.parse(ProblemDocument).status == 404


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_query_distributions_refuses_a_quantile_that_is_not_a_position(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    """A quantile outside (0, 1) names no position, so it is refused rather than clamped."""
    person = stand_manifest.fixture("dev_lead")
    question = _question(stand_manifest, GIT_PR_SIZE, person.uuid)
    question["quantiles"] = [1.5]

    response = api.post(QUERY_DISTRIBUTIONS, json_body={"queries": [question]})
    assert response.status_code == 400, f"status={response.status_code} {response.text[:300]}"
    assert response.parse(ProblemDocument).status == 400


def _tenant_question(manifest: Manifest, metric: str) -> dict[str, JsonValue]:
    start, end = query_window(manifest)
    return {
        "metric": metric,
        "subjects": {"type": "tenant"},
        "time": {"from": start, "to": end},
        "bins": _BINS,
        "quantiles": list(_QUANTILES),
    }


@pytest.mark.reliability
def test_query_distributions_shapes_a_tenant_metric_without_naming_a_subject(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    """One distribution comes back, and it names nobody: the tenant is not a subject.

    The CI connector streams are not part of the sample seed, so this pins that
    exactly one distribution is reported for the one entity a tenant read groups
    by, and that it carries no subject — an unobserved range is a legitimate
    answer here.
    """
    response = api.post(
        QUERY_DISTRIBUTIONS,
        json_body={"queries": [_tenant_question(stand_manifest, CI_RUN_DURATION)]},
    )
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    results = response.parse(DistributionsResponse).results
    assert len(results) == 1
    subjects = results[0].subjects
    assert len(subjects) == 1, f"a tenant question answered {len(subjects)} distributions"
    assert subjects[0].subject is None

    histogram = subjects[0].histogram
    assert histogram is not None, "a question naming bins is answered with a histogram"
    _assert_bins_tile_the_range(histogram)

    quantiles = subjects[0].quantiles
    assert quantiles is not None, "a question naming quantiles is answered with positions"
    assert [quantile.q for quantile in quantiles] == list(_QUANTILES)


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_query_distributions_refuses_subjects_that_are_not_the_metrics_grain(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    """Each surface answers one grain: naming the other side is unanswerable, not empty."""
    person = stand_manifest.fixture("dev_lead")
    cases = {
        "a tenant metric asked about a person": _question(
            stand_manifest, CI_RUN_DURATION, person.uuid
        ),
        "a person metric asked about the tenant": _tenant_question(stand_manifest, GIT_PR_SIZE),
    }

    for named, query in cases.items():
        response = api.post(QUERY_DISTRIBUTIONS, json_body={"queries": [query]})
        assert response.status_code == 400, (
            f"{named} answered {response.status_code}: {response.text[:300]}"
        )
        assert response.parse(ProblemDocument).status == 400
