from __future__ import annotations

from collections.abc import Callable, Iterable
from datetime import date, timedelta

import pytest
from insight_stand import ApiClient, GoldenTasks, Manifest, PersonaSession, analytics_path

from ..schemas import (
    CustomMetric,
    CustomMetricListResponse,
    ExportCustomMetricsResponse,
    ImportCustomMetricsResponse,
    MetricDefinitionListResponse,
    MetricResultsResponse,
    PeriodView,
    ProblemDocument,
    RunResponse,
)
from ..scratch import create_saved_query, custom_metric_body, scratch_metric_identity, track
from . import MAX_QUERY_SPAN_DAYS

METRICS = analytics_path("/v1/metrics")
EXPORT = analytics_path("/v1/metrics/export")
IMPORT = analytics_path("/v1/metrics/import")
DEFINITIONS = analytics_path("/v1/metric-definitions")
RESULTS = analytics_path("/v1/metric-results")
MEASURE = "closes"


def _metric_path(metric_key: str) -> str:
    return analytics_path(f"/v1/metrics/{metric_key}")


def _golden_tasks(manifest: Manifest) -> GoldenTasks:
    golden = manifest.golden_metrics
    if golden is None or golden.tasks is None:
        pytest.skip(f"the manifest at {manifest.source_path} carries no golden task totals")
    if "silver" not in manifest.seeded:
        pytest.skip("the stand was seeded without the silver step")
    return golden.tasks


def _periods(window: str) -> list[tuple[str, str]]:
    start_text, _, end_text = window.partition("..")
    start, end = date.fromisoformat(start_text), date.fromisoformat(end_text)

    periods: list[tuple[str, str]] = []
    while start <= end:
        slice_end = min(start + timedelta(days=MAX_QUERY_SPAN_DAYS), end)
        periods.append((start.isoformat(), slice_end.isoformat()))
        start = slice_end + timedelta(days=1)
    return periods


def _observation_sql(source_key: str, seeded_measure: str) -> str:
    return (
        "SELECT o.tenant_id AS tenant_id, "
        f"'{source_key}' AS source_key, "
        "o.entity_type AS entity_type, "
        "toString(p.person_id) AS entity_id, "
        "o.metric_date AS metric_date, "
        f"'{MEASURE}' AS measure_key, "
        "o.observed_at AS observed_at, "
        "o.value AS value, "
        "o.subject_key AS subject_key, "
        "o.dimensions AS dimensions "
        "FROM insight.task_metric_observations AS o "
        "INNER JOIN identity.person_map AS p ON p.email = o.entity_id "
        f"WHERE o.measure_key = '{seeded_measure}'"
    )


def _metric_body(metric_key: str, source_key: str, seeded_measure: str) -> dict[str, object]:
    body = custom_metric_body(metric_key, source_key)
    body.update(
        label=f"Closes ({seeded_measure})",
        observation_sql=_observation_sql(source_key, seeded_measure),
        measures=[MEASURE],
        inputs=[{"role": "value", "measure_key": MEASURE}],
    )
    return body


def _create(client: ApiClient, tag: str, seeded_measure: str) -> CustomMetric:
    metric_key, source_key = scratch_metric_identity(tag)
    response = client.post(METRICS, json_body=_metric_body(metric_key, source_key, seeded_measure))
    assert response.status_code == 201, f"create: {response.status_code} {response.text[:300]}"
    track(METRICS, "metric_key", metric_key)

    created = response.parse(CustomMetric)
    assert created.metric_key == metric_key
    return created


def _listed(client: ApiClient) -> set[str]:
    response = client.get(METRICS)
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
    return {item.metric_key for item in response.parse(CustomMetricListResponse).items}


def _definitions(client: ApiClient) -> dict[str, str]:
    response = client.get(DEFINITIONS)
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
    return {
        view.metric_key: view.origin.value
        for view in response.parse(MetricDefinitionListResponse).metrics
    }


def _results_body(person: str, start: str, end: str, keys: Iterable[str]) -> dict[str, object]:
    return {
        "entity": {"type": "person", "ids": [person]},
        "period": {"from": start, "to": end},
        "metrics": [{"metric_key": key, "views": [{"view": "period"}]} for key in keys],
    }


def _totals(
    client: ApiClient, person: str, periods: list[tuple[str, str]], keys: list[str]
) -> dict[str, float]:
    totals = dict.fromkeys(keys, 0.0)
    for start, end in periods:
        response = client.post(RESULTS, json_body=_results_body(person, start, end, keys))
        assert response.status_code == 200, (
            f"{start}..{end}: status={response.status_code} {response.text[:300]}"
        )

        for metric in response.parse(MetricResultsResponse).metrics:
            for view in metric.root.views:
                assert isinstance(view.root, PeriodView), type(view.root).__name__
                for value in view.root.values:
                    assert value.entity_id == person
                    totals[metric.root.metric_key] += value.value or 0
    return totals


def _sql_oracle(client: ApiClient, observation_sql: str, person: str, window: str) -> object:
    query = create_saved_query(
        client,
        "an3-oracle",
        f"SELECT sum(value) AS closed FROM ({observation_sql}) "
        "WHERE tenant_id = {tenant:String} "
        f"AND entity_id = '{person}' "
        "AND metric_date BETWEEN toDate(splitByString('..', {period:String})[1]) "
        "AND toDate(splitByString('..', {period:String})[2])",
    )
    try:
        ran = client.post(
            analytics_path(f"/v1/queries/{query.id}/run"), json_body={"period": window}
        )
        assert ran.status_code == 200, f"oracle run: {ran.status_code} {ran.text[:300]}"
        rows = ran.parse(RunResponse).rows
    finally:
        client.delete(analytics_path(f"/v1/queries/{query.id}"))

    assert len(rows) == 1, rows
    return rows[0]["closed"]


@pytest.mark.requires_seed("dev_lead", "sales_lead", "development_ic")
@pytest.mark.reliability
def test_a_custom_metric_reaches_the_dashboard_with_the_numbers_its_sql_yields(
    session_for: Callable[[str], PersonaSession], stand_manifest: Manifest
) -> None:
    tasks = _golden_tasks(stand_manifest)
    periods = _periods(tasks.window)
    author = session_for("dev_lead").client
    reader = session_for("sales_lead").client
    ic = stand_manifest.fixture("development_ic")
    golden = tasks.per_person[ic.email]

    metric = _create(author, "an3", "tasks_closed")
    key = metric.metric_key
    try:
        assert key in _listed(author)
        fetched = author.get(_metric_path(key))
        assert fetched.status_code == 200, f"read: {fetched.status_code} {fetched.text[:300]}"
        assert fetched.parse(CustomMetric).observation_sql == metric.observation_sql

        assert _definitions(author).get(key) == "custom"
        assert _definitions(reader).get(key) == "custom"

        oracle = _sql_oracle(author, metric.observation_sql, ic.uuid, tasks.window)
        assert oracle == golden.tasks_closed
        assert _totals(author, ic.uuid, periods, [key]) == {key: golden.tasks_closed}

        narrowed = _metric_body(key, metric.source_key, "closed_task")
        updated = author.put(_metric_path(key), json_body=narrowed)
        assert updated.status_code == 200, f"update: {updated.status_code} {updated.text[:300]}"
        assert updated.parse(CustomMetric).observation_sql == narrowed["observation_sql"]

        after = _totals(author, ic.uuid, periods, [key])
        assert golden.tasks_closed - after[key] == golden.bugs_fixed, after

        deleted = author.delete(_metric_path(key))
        assert deleted.status_code == 204, f"delete: {deleted.status_code} {deleted.text[:300]}"

        assert key not in _definitions(author)
        assert key not in _definitions(reader)
        start, end = periods[-1]
        refused = author.post(RESULTS, json_body=_results_body(ic.uuid, start, end, [key]))
        assert refused.status_code == 400, f"{refused.status_code} {refused.text[:300]}"
        assert refused.parse(ProblemDocument).status == 400
    finally:
        author.delete(_metric_path(key))


@pytest.mark.requires_seed("dev_lead", "development_ic")
@pytest.mark.reliability
def test_custom_metrics_survive_export_wipe_and_import(
    session_for: Callable[[str], PersonaSession], stand_manifest: Manifest
) -> None:
    tasks = _golden_tasks(stand_manifest)
    periods = _periods(tasks.window)
    author = session_for("dev_lead").client
    ic = stand_manifest.fixture("development_ic")
    golden = tasks.per_person[ic.email]

    made = [
        _create(author, "an4_closes", "tasks_closed"),
        _create(author, "an4_bugs", "bugs_fixed"),
    ]
    keys = [metric.metric_key for metric in made]
    try:
        before = _totals(author, ic.uuid, periods, keys)
        assert before == dict(zip(keys, [golden.tasks_closed, golden.bugs_fixed], strict=True))

        exported = author.get(EXPORT)
        if exported.status_code == 500:
            pytest.xfail(
                f"constructorfabric/insight#2360 — export answers 500: {exported.text[:200]}"
            )
        assert exported.status_code == 200, f"export: {exported.status_code} {exported.text[:300]}"
        graphs = {
            metric.metric_key: metric
            for metric in exported.parse(ExportCustomMetricsResponse).metrics
            if metric.metric_key in keys
        }
        assert sorted(graphs) == sorted(keys)

        for key in keys:
            wiped = author.delete(_metric_path(key))
            assert wiped.status_code == 204, f"delete: {wiped.status_code} {wiped.text[:300]}"
        assert not set(keys) & _listed(author)

        body = {"metrics": [graphs[key].model_dump(mode="json", exclude_none=True) for key in keys]}
        imported = author.post(IMPORT, json_body=body)
        assert imported.status_code == 200, f"import: {imported.status_code} {imported.text[:300]}"
        landed = imported.parse(ImportCustomMetricsResponse)
        assert (landed.imported, landed.skipped) == (2, [])

        assert set(keys) <= _listed(author)
        for key in keys:
            restored = author.get(_metric_path(key)).parse(CustomMetric)
            assert restored.model_copy(update={"origin": None}) == graphs[key]

        assert _totals(author, ic.uuid, periods, keys) == before

        again = author.post(IMPORT, json_body=body)
        assert again.status_code == 200, f"re-import: {again.status_code} {again.text[:300]}"
        skipped = again.parse(ImportCustomMetricsResponse)
        assert (skipped.imported, sorted(skipped.skipped)) == (0, sorted(keys))
    finally:
        for key in keys:
            author.delete(_metric_path(key))
