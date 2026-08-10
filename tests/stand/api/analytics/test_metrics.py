"""The `/v1/metrics` path group on analytics — custom-metric CRUD + export/import.

    GET    /v1/metrics                200 list
    POST   /v1/metrics                201 · 400 invalid graph / observation SQL · 409 duplicate
    GET    /v1/metrics/export         200 · portable graphs, no tenant/origin
    POST   /v1/metrics/import         200 imported/skipped · 400 invalid
    GET    /v1/metrics/{metric_key}   200 · 404 unknown
    PUT    /v1/metrics/{metric_key}   200 · 400 invalid · 404 unknown
    DELETE /v1/metrics/{metric_key}   204 · 404 unknown

`metric_key` is a dotted `family.name`, not a UUID: an unknown one is 404, so
these routes are deliberately absent from the non-uuid 400 sweep in
`test_request_contracts.py`. The wrong-media-type (415) and off-schema (422)
halves for the body routes are swept there over every route at once.

Every custom metric is `origin='custom'` and tenant-scoped; a builtin key is
invisible here and reads back 404. Creating one requires observation SQL that
passes the single-SELECT gate AND emits the observation contract columns under a
ClickHouse LIMIT-0 probe — `scratch.SCRATCH_OBSERVATION_SQL` is one that does.

The 401 half is in `test_gateway.py`, swept over every operation at once.
"""

from __future__ import annotations

from typing import Any

import pytest
from insight_stand import ApiClient, analytics_path

from ..schemas import (
    CustomMetric,
    CustomMetricListResponse,
    ExportCustomMetricsResponse,
    ImportCustomMetricsResponse,
    ProblemDocument,
)
from ..scratch import (
    UNKNOWN_METRIC_KEY,
    create_custom_metric,
    custom_metric_body,
    scratch_metric_identity,
    track,
)

METRICS = analytics_path("/v1/metrics")
EXPORT = analytics_path("/v1/metrics/export")
IMPORT = analytics_path("/v1/metrics/import")


def _metric_path(metric_key: str) -> str:
    return analytics_path(f"/v1/metrics/{metric_key}")


def _listed_keys(api: ApiClient) -> set[str]:
    """Every custom-metric key the listing reports, validated on the way through."""
    response = api.get(METRICS)
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
    return {item.metric_key for item in response.parse(CustomMetricListResponse).items}


def test_list_metrics_200(api: ApiClient, scratch_custom_metric: CustomMetric) -> None:
    assert scratch_custom_metric.metric_key in _listed_keys(api)


def test_custom_metric_create_get_update_delete_round_trip(api: ApiClient) -> None:
    """One cycle: create → read → update → delete → gone.

    Asserted as a cycle rather than as separate cases for the same reason the
    saved-query round trip is: a create that leaks its row and a delete that runs
    against a row it did not make are the two ways this coverage rots, and a
    single cycle can do neither.
    """
    created = create_custom_metric(api, "roundtrip")
    metric_key = created.metric_key

    fetched = api.get(_metric_path(metric_key))
    assert fetched.status_code == 200, f"read back: {fetched.status_code} {fetched.text[:300]}"
    assert fetched.parse(CustomMetric).metric_key == metric_key

    body = custom_metric_body(metric_key, created.source_key)
    body["label"] = "updated by the stand suite"
    updated = api.put(_metric_path(metric_key), json_body=body)
    assert updated.status_code == 200, f"update: {updated.status_code} {updated.text[:300]}"
    reloaded = updated.parse(CustomMetric)
    assert reloaded.metric_key == metric_key, "the path key is authoritative on update"
    assert reloaded.label == "updated by the stand suite"

    deleted = api.delete(_metric_path(metric_key))
    assert deleted.status_code == 204, f"delete: {deleted.status_code} {deleted.text[:300]}"

    assert api.get(_metric_path(metric_key)).status_code == 404
    assert metric_key not in _listed_keys(api), "a deleted custom metric is still listed"


def test_get_metric_404_unknown(api: ApiClient) -> None:
    response = api.get(_metric_path(UNKNOWN_METRIC_KEY))
    assert response.status_code == 404, f"status={response.status_code} {response.text[:300]}"
    assert response.parse(ProblemDocument).status == 404


def test_update_metric_404_unknown(api: ApiClient) -> None:
    """A valid body against an absent key is 404 — validation passes first.

    The body is well-formed on purpose: `validate_graph` runs before the row is
    looked up, so an invalid body would answer 400 for the wrong reason and never
    reach the not-found path this case is about.
    """
    response = api.put(
        _metric_path(UNKNOWN_METRIC_KEY),
        json_body=custom_metric_body(UNKNOWN_METRIC_KEY, "absent_source"),
    )
    assert response.status_code == 404, f"status={response.status_code} {response.text[:300]}"
    assert response.parse(ProblemDocument).status == 404


def test_delete_metric_404_unknown(api: ApiClient) -> None:
    response = api.delete(_metric_path(UNKNOWN_METRIC_KEY))
    assert response.status_code == 404, f"status={response.status_code} {response.text[:300]}"
    assert response.parse(ProblemDocument).status == 404


def test_create_metric_409_on_a_duplicate_key(api: ApiClient) -> None:
    """A second create of the same key conflicts rather than overwriting.

    The one conflict path on this surface, and the reason `POST /v1/metrics`
    does not block 409 in the coverage gate. Cleaned up in a `finally` so the
    conflicting attempt cannot leak the row the first create made.
    """
    created = create_custom_metric(api, "conflict")
    body = custom_metric_body(created.metric_key, created.source_key)
    try:
        again = api.post(METRICS, json_body=body)
        assert again.status_code == 409, (
            f"a duplicate metric_key was accepted ({again.status_code}): {again.text[:300]}"
        )
        assert again.parse(ProblemDocument).status == 409
    finally:
        api.delete(_metric_path(created.metric_key))


@pytest.mark.parametrize(
    ("label", "mutation"),
    [
        ("bad metric_key", {"metric_key": "NoDot"}),
        ("non-single-select observation_sql", {"observation_sql": "DROP TABLE t"}),
        ("observation_sql omitting a contract column", {"observation_sql": "SELECT 1 AS one"}),
    ],
    ids=["bad-key", "not-a-read", "missing-column"],
)
def test_create_metric_400_for_an_invalid_graph(
    api: ApiClient, label: str, mutation: dict[str, Any]
) -> None:
    """The write gate, at the point a metric is stored.

    Two gates answer 400 here and both matter. The pure `validate_graph` rejects
    a malformed key or a statement that is not a single read; the ClickHouse
    LIMIT-0 probe rejects SQL that parses but does not emit the observation
    contract — a source that would render nothing had it been accepted.
    """
    metric_key, source_key = scratch_metric_identity("invalid")
    body = custom_metric_body(metric_key, source_key)
    body.update(mutation)

    response = api.post(METRICS, json_body=body)
    assert response.status_code == 400, (
        f"a {label} was accepted as a custom metric ({response.status_code}): {response.text[:300]}"
    )


def test_export_metrics_200(api: ApiClient, scratch_custom_metric: CustomMetric) -> None:
    """The tenant's custom graphs come back portable — the scratch one among them."""
    response = api.get(EXPORT)
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
    exported = response.parse(ExportCustomMetricsResponse)
    assert scratch_custom_metric.metric_key in {metric.metric_key for metric in exported.metrics}


def test_import_metrics_200_imports_then_skips_a_duplicate(api: ApiClient) -> None:
    """Import lands new graphs and is idempotent — a re-import skips by key.

    Both branches of the response in one cycle: the first import reports the key
    as imported, the second reports it skipped without a second row. Cleaned up
    in a `finally` so the imported metric cannot leak.
    """
    metric_key, source_key = scratch_metric_identity("import")
    body = {"metrics": [custom_metric_body(metric_key, source_key)]}
    track(METRICS, "metric_key", metric_key)
    try:
        first = api.post(IMPORT, json_body=body)
        assert first.status_code == 200, f"import: {first.status_code} {first.text[:300]}"
        landed = first.parse(ImportCustomMetricsResponse)
        assert landed.imported == 1 and landed.skipped == [], (
            f"first import did not land exactly one metric: {first.text[:300]}"
        )

        again = api.post(IMPORT, json_body=body)
        assert again.status_code == 200, f"re-import: {again.status_code} {again.text[:300]}"
        skipped = again.parse(ImportCustomMetricsResponse)
        assert skipped.imported == 0 and skipped.skipped == [metric_key], (
            f"a re-import of an existing key was not skipped: {again.text[:300]}"
        )
    finally:
        api.delete(_metric_path(metric_key))


def test_import_metrics_400_for_an_invalid_graph(api: ApiClient) -> None:
    """One malformed graph in the batch refuses the whole import.

    Every graph is validated before any is written, so a bad key rejects the
    request rather than partially applying it.
    """
    metric_key, source_key = scratch_metric_identity("import-bad")
    body = custom_metric_body(metric_key, source_key)
    body["metric_key"] = "NoDot"

    response = api.post(IMPORT, json_body={"metrics": [body]})
    assert response.status_code == 400, f"status={response.status_code} {response.text[:300]}"


def test_update_metric_400_for_an_invalid_graph(
    api: ApiClient, scratch_custom_metric: CustomMetric
) -> None:
    """An update revalidates the graph — a stored metric that passed once can be
    rewritten, so the gate has to run again on the new SQL."""
    body = custom_metric_body(scratch_custom_metric.metric_key, scratch_custom_metric.source_key)
    body["observation_sql"] = "DROP TABLE t"

    response = api.put(_metric_path(scratch_custom_metric.metric_key), json_body=body)
    assert response.status_code == 400, (
        f"a non-read statement was accepted on update ({response.status_code}): "
        f"{response.text[:300]}"
    )
