"""`GET /v1/catalog/metrics` — what the semantic definitions can be asked.

    GET /v1/catalog/metrics  200 the metrics, and the questions each admits

The 401 half is in `test_gateway.py`, swept over every operation at once.

No seed and no manifest. The document is a projection of definitions compiled
into the service, so it is the same on a full stand and an empty one — and the
only thing worth asserting about it is that it does not contradict itself. Every
claim below is checked against another field of the SAME response: a metric that
advertises a distribution must be the kind of computation that has one, a metric
that advertises a combined fold must declare a dimension to fold into, and a
metric that advertises a cohort comparison must declare the cohort it reads.

Assertions stay off the metric keys on purpose. Which metrics ship is a product
decision that moves; that the advertisement is answerable is the contract.
"""

from __future__ import annotations

import pytest
from insight_stand import ApiClient, analytics_path

from ..schemas.analytics import MetricCatalogResponse

CATALOG_METRICS = analytics_path("/v1/catalog/metrics")

#: The computations taken over a measure's per-row values — the ones that have a
#: distribution at all.
_DISTRIBUTABLE = frozenset({"percentile", "stddev"})

#: How many drilldown pages a computation advertises, as `(minimum, maximum)`
#: with `None` for unbounded: a ratio names its two sides, a derived metric one
#: page per input its definition folds — never fewer than the two that make it
#: composed — and every other computation the single measure it reads.
_INPUTS_PER_COMPUTATION: dict[str, tuple[int, int | None]] = {
    "ratio": (2, 2),
    "derived": (2, None),
}
_ONE_MEASURE: tuple[int, int | None] = (1, 1)

TENANT = {"type": "tenant"}
COHORT = {"type": "cohort"}


def _advertises_one_page_per_input(computation: str, pages: int) -> bool:
    minimum, maximum = _INPUTS_PER_COMPUTATION.get(computation, _ONE_MEASURE)
    return pages >= minimum and (maximum is None or pages <= maximum)


@pytest.mark.reliability
def test_catalog_metrics_answers_the_shape_its_contract_declares(api: ApiClient) -> None:
    """Every metric parses against the published document, and the keys are distinct."""
    response = api.get(CATALOG_METRICS)
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    catalogued = response.parse(MetricCatalogResponse)
    assert catalogued.metrics, "the catalogue advertised no metric at all"

    keys = [metric.key for metric in catalogued.metrics]
    assert len(keys) == len(set(keys)), f"a metric key is catalogued twice: {keys}"
    assert keys == sorted(keys), f"the catalogue is not in key order: {keys}"


@pytest.mark.reliability
def test_every_question_a_metric_advertises_agrees_with_the_rest_of_its_entry(
    api: ApiClient,
) -> None:
    """The advertisement is internally consistent, judged from the response alone."""
    response = api.get(CATALOG_METRICS)
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
    response.parse(MetricCatalogResponse)

    for metric in response.json()["metrics"]:
        key = metric["key"]
        computation = metric["computation"]["type"]
        splittable = bool(metric["dimensions"])
        questions = metric["questions"]

        assert questions["distributions"]["admitted"] == (computation in _DISTRIBUTABLE), (
            f"`{key}` is a {computation} and advertises "
            f"{questions['distributions']['admitted']} for a distribution"
        )

        values = questions["values"]
        assert values["grains"] == ["total", "day", "week", "month"], (
            f"`{key}` advertises grains {values['grains']}"
        )
        assert values["split"] == splittable, (
            f"`{key}` advertises split={values['split']} with {len(metric['dimensions'])} "
            f"dimensions"
        )
        assert "per_subject" in values["folds"], f"`{key}` advertises folds {values['folds']}"
        assert ("combined" in values["folds"]) == splittable, (
            f"`{key}` advertises a combined fold with nothing to fold into: {values['folds']}"
        )
        assert values["compare"], f"`{key}` advertises no comparable window"

        populations = questions["comparisons"]["populations"]
        assert TENANT in populations, f"`{key}` advertises populations {populations}"
        assert (COHORT in populations) == ("cohort_key" in metric), (
            f"`{key}` advertises {populations} while its cohort_key is {metric.get('cohort_key')!r}"
        )

        inputs = questions["rows"]["inputs"]
        assert _advertises_one_page_per_input(computation, len(inputs)), (
            f"`{key}` is a {computation} and names {inputs} as the pages behind it"
        )
        assert len(inputs) == len(set(inputs)), f"`{key}` names one page twice: {inputs}"


@pytest.mark.security
def test_catalog_metrics_discloses_no_value_of_any_dimension(api: ApiClient) -> None:
    """A dimension is advertised by key and label only — never by what it holds."""
    response = api.get(CATALOG_METRICS)
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    for metric in response.json()["metrics"]:
        for dimension in metric["dimensions"]:
            assert sorted(dimension) == ["key", "label"], (
                f"`{metric['key']}` advertises a dimension carrying more than its name: {dimension}"
            )
