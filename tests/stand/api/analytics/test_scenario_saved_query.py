from __future__ import annotations

from collections.abc import Callable

import pytest
from insight_stand import ApiClient, GoldenTasks, Manifest, PersonaSession, analytics_path

from ..schemas import ProblemDocument, RunResponse, SavedQuery, SavedQueryListResponse
from ..scratch import SCRATCH_QUERY_REF, create_saved_query

QUERIES = analytics_path("/v1/queries")


def _query_path(query_id: object, suffix: str = "") -> str:
    return analytics_path(f"/v1/queries/{query_id}{suffix}")


def _golden_tasks(manifest: Manifest) -> GoldenTasks:
    golden = manifest.golden_metrics
    if golden is None or golden.tasks is None:
        pytest.skip(f"the manifest at {manifest.source_path} carries no golden task totals")
    if "silver" not in manifest.seeded:
        pytest.skip("the stand was seeded without the silver step")
    return golden.tasks


def _closes_sql(email: str, measure_key: str) -> str:
    return (
        "SELECT sum(value) AS closed FROM insight.task_metric_observations "
        "WHERE tenant_id = {tenant:String} "
        f"AND entity_id = '{email}' "
        f"AND measure_key = '{measure_key}' "
        "AND metric_date BETWEEN toDate(splitByString('..', {period:String})[1]) "
        "AND toDate(splitByString('..', {period:String})[2])"
    )


def _listed(client: ApiClient) -> set[str]:
    response = client.get(QUERIES)
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
    return {item.name for item in response.parse(SavedQueryListResponse).items}


def _run(client: ApiClient, query_id: object, period: str) -> list[object]:
    response = client.post(_query_path(query_id, "/run"), json_body={"period": period})
    assert response.status_code == 200, f"run: {response.status_code} {response.text[:300]}"
    return response.parse(RunResponse).rows


@pytest.mark.requires_seed("dev_lead", "sales_lead")
@pytest.mark.reliability
def test_a_saved_query_answers_the_seeded_closes_and_is_shared_across_the_tenant(
    session_for: Callable[[str], PersonaSession], stand_manifest: Manifest
) -> None:
    tasks = _golden_tasks(stand_manifest)
    author = session_for("dev_lead")
    colleague = session_for("sales_lead").client
    expected = tasks.per_person[author.email]

    query = create_saved_query(
        author.client, "an1-closes", _closes_sql(author.email, "tasks_closed")
    )
    try:
        assert _run(author.client, query.id, tasks.window) == [{"closed": expected.tasks_closed}]

        assert query.name in _listed(colleague)
        shared = colleague.get(_query_path(query.id))
        assert shared.status_code == 200, f"{shared.status_code} {shared.text[:300]}"
        assert shared.parse(SavedQuery).sql == query.sql

        narrowed_sql = _closes_sql(author.email, "bugs_fixed")
        updated = author.client.put(
            _query_path(query.id),
            json_body={"name": query.name, "description": "bugs only", "sql": narrowed_sql},
        )
        assert updated.status_code == 200, f"update: {updated.status_code} {updated.text[:300]}"
        assert updated.parse(SavedQuery).sql == narrowed_sql

        assert _run(author.client, query.id, tasks.window) == [{"closed": expected.bugs_fixed}]
        assert colleague.get(_query_path(query.id)).parse(SavedQuery).sql == narrowed_sql

        deleted = author.client.delete(_query_path(query.id))
        assert deleted.status_code == 204, f"delete: {deleted.status_code} {deleted.text[:300]}"

        assert query.name not in _listed(colleague)
        gone = colleague.get(_query_path(query.id))
        assert gone.status_code == 404, f"{gone.status_code} {gone.text[:300]}"
        assert gone.parse(ProblemDocument).status == 404
    finally:
        author.client.delete(_query_path(query.id))


@pytest.mark.requires_seed("dev_lead", "other_tenant_lead")
@pytest.mark.security
def test_another_tenant_can_neither_see_nor_touch_a_saved_query(
    session_for: Callable[[str], PersonaSession],
    other_tenant_session: PersonaSession,
    stand_manifest: Manifest,
) -> None:
    tasks = _golden_tasks(stand_manifest)
    author = session_for("dev_lead")
    outsider = other_tenant_session.client
    sql = _closes_sql(author.email, "tasks_closed")

    query = create_saved_query(author.client, "an1-tenant", sql)
    try:
        assert query.name not in _listed(outsider)

        attempts = {
            "read": outsider.get(_query_path(query.id)),
            "run": outsider.post(_query_path(query.id, "/run"), json_body={"period": tasks.window}),
            "update": outsider.put(_query_path(query.id), json_body={"sql": SCRATCH_QUERY_REF}),
            "delete": outsider.delete(_query_path(query.id)),
        }
        for action, response in attempts.items():
            assert response.status_code == 404, (
                f"another tenant's {action} answered {response.status_code}: {response.text[:300]}"
            )
            assert response.parse(ProblemDocument).status == 404

        kept = author.client.get(_query_path(query.id))
        assert kept.status_code == 200, f"{kept.status_code} {kept.text[:300]}"
        assert kept.parse(SavedQuery).sql == sql
        assert _run(author.client, query.id, tasks.window) == [
            {"closed": tasks.per_person[author.email].tasks_closed}
        ]
    finally:
        author.client.delete(_query_path(query.id))
