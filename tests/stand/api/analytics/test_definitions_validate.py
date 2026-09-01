"""`POST /v1/definitions/validate` — judging definitions without keeping them.

    POST /v1/definitions/validate  200 valid · 200 invalid, with every offender

The 401 half is in `test_gateway.py` and the 415 half in
`test_request_contracts.py`, both swept over every operation at once.

Two things make this endpoint worth a deployed case rather than a unit test. It
answers 200 for a set that breaks every rule — the outcome is the payload, so a
proxy or handler turning a refusal into a status code would be a real regression.
And it writes nothing: the probe definitions below are submitted under keys no
seed uses, and the case that follows asks the catalogue whether either of them
became a metric.
"""

from __future__ import annotations

import pytest
from insight_stand import ApiClient, analytics_path
from insight_stand.api import JsonValue

from ..schemas.analytics import MetricCatalogResponse, ValidateDefinitionsResponse

VALIDATE_DEFINITIONS = analytics_path("/v1/definitions/validate")
CATALOG_METRICS = analytics_path("/v1/catalog/metrics")

#: A key no shipped definition holds, so a collision would be this request's.
PROBE_MEASURE = "stand_probe_lines_touched"
PROBE_METRIC = "stand_probe.lines_touched"

#: Reads a dataset and fields the shipped catalog carries, which is what makes
#: the pair valid without seeding anything.
_VALID_MEASURE: dict[str, JsonValue] = {
    "key": PROBE_MEASURE,
    "dataset": "git_commits",
    "aggregation": "sum",
    "value_expr": "lines_added + lines_removed",
    "event_time": "authored_at",
    "entity": "author_email",
    "dimensions": [{"key": "repository", "value_field": "repository"}],
}

_VALID_METRIC: dict[str, JsonValue] = {
    "key": PROBE_METRIC,
    "computation": {"type": "direct", "measure": PROBE_MEASURE},
    "format": "integer",
    "direction": "neutral",
    "entity_type": "person",
}

#: Two rules broken at once, in the two halves of the request, so the answer has
#: to accumulate rather than stop at the first offender: the measure reads a
#: dataset the catalog does not have, and the metric reads a measure nothing
#: defines.
_BROKEN_MEASURE: dict[str, JsonValue] = {**_VALID_MEASURE, "dataset": "no_such_dataset"}
_BROKEN_METRIC: dict[str, JsonValue] = {
    **_VALID_METRIC,
    "computation": {"type": "direct", "measure": "no_such_measure"},
}


@pytest.mark.reliability
def test_a_valid_pair_is_judged_valid_against_the_shipped_definitions(api: ApiClient) -> None:
    """A measure and the metric reading it validate as one set with what ships."""
    response = api.post(
        VALIDATE_DEFINITIONS,
        json_body={"measures": [_VALID_MEASURE], "metrics": [_VALID_METRIC]},
    )
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    judged = response.parse(ValidateDefinitionsResponse)
    assert judged.valid, f"a valid pair was refused: {judged.errors}"
    assert judged.errors == []


@pytest.mark.reliability
def test_a_broken_pair_is_reported_offender_by_offender_and_still_answers_200(
    api: ApiClient,
) -> None:
    """Validation's outcome is the body, and it names every rule broken, not the first."""
    response = api.post(
        VALIDATE_DEFINITIONS,
        json_body={"measures": [_BROKEN_MEASURE], "metrics": [_BROKEN_METRIC]},
    )
    assert response.status_code == 200, (
        f"an invalid set answered {response.status_code} rather than reporting itself: "
        f"{response.text[:300]}"
    )

    judged = response.parse(ValidateDefinitionsResponse)
    assert not judged.valid
    assert {error.kind for error in judged.errors} == {"dataset_not_found", "measure_not_found"}, (
        f"expected both halves to be judged, got {judged.errors}"
    )
    assert all(error.message for error in judged.errors), (
        f"an offender was reported without saying what it broke: {judged.errors}"
    )


@pytest.mark.security
def test_a_metric_that_only_validated_does_not_join_the_catalogue(api: ApiClient) -> None:
    """The dry run keeps nothing — the probe metric is unknown to discovery afterwards."""
    accepted = api.post(
        VALIDATE_DEFINITIONS,
        json_body={"measures": [_VALID_MEASURE], "metrics": [_VALID_METRIC]},
    )
    assert accepted.status_code == 200, f"status={accepted.status_code} {accepted.text[:300]}"
    assert accepted.parse(ValidateDefinitionsResponse).valid

    catalogued = api.get(CATALOG_METRICS)
    assert catalogued.status_code == 200, f"status={catalogued.status_code} {catalogued.text[:300]}"
    keys = [metric.key for metric in catalogued.parse(MetricCatalogResponse).metrics]
    assert PROBE_METRIC not in keys, f"a validated metric was installed: {keys}"
