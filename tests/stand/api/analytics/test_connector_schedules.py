from __future__ import annotations

from collections.abc import Iterator

import pytest
from insight_stand import PersonaSession, analytics_path

from ..operations import SOME_CONNECTOR, SOME_SOURCE_ID
from ..schemas import ConnectorHealthResponse, ProblemDocument
from ..schemas.analytics import ScheduleResponse

READER = analytics_path("/v1/connector-schedules")
UNKNOWN = analytics_path(f"/v1/connectors/{SOME_CONNECTOR}/{SOME_SOURCE_ID}/schedule")


@pytest.fixture(scope="module")
def schedule_path(admin_operator_session: PersonaSession) -> Iterator[str]:
    client = admin_operator_session.client
    response = client.get(analytics_path("/v1/connector-health"))
    assert response.status_code == 200, response.text
    summary = response.parse(ConnectorHealthResponse)
    instances = [
        row
        for row in summary.connectors
        if row.configured and row.source_id and row.connector.startswith("stand-scratch-")
    ]
    assert instances, (
        "Seed a synthetic stand-scratch-* configured instance before running schedule lifecycle tests"
    )
    assert len(instances) == 1
    row = instances[0]
    path = analytics_path(f"/v1/connectors/{row.connector}/{row.source_id}/schedule")
    before = client.get(path)
    assert before.status_code == 200, before.text
    initial = before.parse(ScheduleResponse)
    assert initial.status.value == "inherited"
    assert initial.requested is None or initial.requested.crons == []
    try:
        yield path
    finally:
        current = client.get(path)
        assert current.status_code == 200, current.text
        current.parse(ScheduleResponse)
        cleared = client.delete(path, headers={"If-Match": current.headers["etag"]})
        assert cleared.status_code == 200, cleared.text
        restored = cleared.parse(ScheduleResponse)
        assert restored.status.value == "inherited"
        assert restored.requested is None or restored.requested.crons == []


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_schedule_lifecycle_preserves_revisions_and_restores_inheritance(
    admin_operator_session: PersonaSession, schedule_path: str
) -> None:
    client = admin_operator_session.client
    initial_response = client.get(schedule_path)
    assert initial_response.status_code == 200, initial_response.text
    initial = initial_response.parse(ScheduleResponse)
    revision = initial.revision
    initial_etag = f'"{revision}"'
    assert initial_response.headers["etag"] == initial_etag
    assert initial.editable
    try:
        saved = client.put(
            schedule_path, headers={"If-Match": initial_etag}, json_body={"crons": ["0 6 * * *"]}
        )
        assert saved.status_code == 200, saved.text
        requested = saved.parse(ScheduleResponse)
        assert requested.revision == revision + 1
        assert requested.status.value == "pending"
        assert requested.requested is not None
        assert requested.requested.crons == ["0 6 * * *"]
        assert saved.headers["etag"] == f'"{revision + 1}"'
        pending_response = client.get(schedule_path)
        assert pending_response.status_code == 200, pending_response.text
        pending = pending_response.parse(ScheduleResponse)
        assert pending == requested
        assert pending_response.headers["etag"] == saved.headers["etag"]
        for method in ["PUT", "DELETE"]:
            stale = client.request(
                method,
                schedule_path,
                headers={"If-Match": initial_etag},
                json_body={"crons": ["0 9 * * *"]} if method == "PUT" else None,
            )
            assert stale.status_code == 412, stale.text
            assert stale.headers["etag"] == saved.headers["etag"]
            assert stale.parse(ProblemDocument).context["revision"] == requested.revision
    finally:
        current = client.get(schedule_path)
        assert current.status_code == 200, current.text
        current.parse(ScheduleResponse)
        deleted = client.delete(schedule_path, headers={"If-Match": current.headers["etag"]})
        assert deleted.status_code == 200, deleted.text
        inherited = deleted.parse(ScheduleResponse)
        assert inherited.status.value == "inherited"
        assert inherited.revision == revision + 2
        assert inherited.requested is not None
        assert inherited.requested.crons == []
        repeated = client.delete(schedule_path, headers={"If-Match": deleted.headers["etag"]})
        assert repeated.status_code == 200, repeated.text
        assert repeated.parse(ScheduleResponse).revision == inherited.revision
        assert repeated.headers["etag"] == deleted.headers["etag"]


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
@pytest.mark.parametrize("method", ["PUT", "DELETE"])
def test_writes_require_if_match(
    admin_operator_session: PersonaSession, schedule_path: str, method: str
) -> None:
    response = admin_operator_session.client.request(
        method, schedule_path, json_body={"crons": ["0 6 * * *"]}
    )
    assert response.status_code == 428, response.text
    assert response.parse(ProblemDocument).status == 428


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
@pytest.mark.parametrize("cron", ["not a cron", "* * * * *"])
def test_invalid_crons_report_field_violations(
    admin_operator_session: PersonaSession, schedule_path: str, cron: str
) -> None:
    client = admin_operator_session.client
    current = client.get(schedule_path)
    assert current.status_code == 200, current.text
    current.parse(ScheduleResponse)
    response = client.put(
        schedule_path, headers={"If-Match": current.headers["etag"]}, json_body={"crons": [cron]}
    )
    assert response.status_code == 400, response.text
    problem = response.parse(ProblemDocument)
    violations = problem.context["field_violations"]
    assert isinstance(violations, list) and violations
    assert all(
        isinstance(v, dict) and str(v.get("field", "")).startswith("crons") and v.get("description")
        for v in violations
    )


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_put_requires_json(admin_operator_session: PersonaSession, schedule_path: str) -> None:
    client = admin_operator_session.client
    current = client.get(schedule_path)
    assert current.status_code == 200, current.text
    current.parse(ScheduleResponse)
    response = client.put(
        schedule_path,
        headers={"If-Match": current.headers["etag"], "Content-Type": "text/plain"},
        content='{"crons":["0 6 * * *"]}',
    )
    assert response.status_code == 415, response.text
    assert response.parse(ProblemDocument).status == 415


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
@pytest.mark.parametrize("method", ["GET", "PUT", "DELETE"])
def test_an_unconfigured_instance_is_not_found(
    admin_operator_session: PersonaSession, method: str
) -> None:
    response = admin_operator_session.client.request(
        method,
        UNKNOWN,
        headers={"If-Match": '"0"'},
        json_body={"crons": ["0 6 * * *"]} if method == "PUT" else None,
    )
    assert response.status_code == 404, response.text
    assert response.parse(ProblemDocument).status == 404


@pytest.mark.security
@pytest.mark.parametrize("method", ["GET", "PUT", "DELETE"])
def test_a_non_admin_cannot_read_or_edit_a_schedule(
    lead_session: PersonaSession, method: str
) -> None:
    response = lead_session.client.request(
        method,
        UNKNOWN,
        headers={"If-Match": '"0"'},
        json_body={"crons": ["0 6 * * *"]} if method == "PUT" else None,
    )
    assert response.status_code == 403, response.text
    assert response.parse(ProblemDocument).status == 403


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.security
def test_a_user_session_cannot_read_reconcile_schedules(
    admin_operator_session: PersonaSession,
) -> None:
    response = admin_operator_session.client.get(READER)
    assert response.status_code == 403, response.text
    assert response.parse(ProblemDocument).status == 403
