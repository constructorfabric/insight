from __future__ import annotations

from collections.abc import Iterator

import pytest
from insight_stand import PersonaSession, connectors_path

from ..operations import SOME_CONNECTOR, SOME_SOURCE_ID
from ..schemas import ProblemDocument
from ..schemas.connectors import ScheduleResponse

UNKNOWN = connectors_path(f"/v1/connectors/{SOME_CONNECTOR}/{SOME_SOURCE_ID}/schedule")
DISABLED = "cluster connection is unavailable"


@pytest.fixture(scope="module")
def schedule_path(admin_operator_session: PersonaSession) -> Iterator[str]:
    client = admin_operator_session.client
    path = connectors_path("/v1/connectors/stand-scratch-connector-schedules/main/schedule")
    before = client.get(path)
    assert before.status_code == 200, before.text
    initial = before.parse(ScheduleResponse)
    assert initial.live is None
    assert initial.inherited is None
    yield path
    current = client.get(path)
    assert current.status_code == 200, current.text
    restored = current.parse(ScheduleResponse)
    assert restored.revision == initial.revision
    assert restored.requested == initial.requested


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_unavailable_cluster_preserves_the_request_and_revision(
    admin_operator_session: PersonaSession, schedule_path: str
) -> None:
    client = admin_operator_session.client
    initial_response = client.get(schedule_path)
    initial = initial_response.parse(ScheduleResponse)
    for method in ["PUT", "DELETE"]:
        refused = client.request(
            method,
            schedule_path,
            headers={"If-Match": initial_response.headers["etag"]},
            json_body={"crons": ["0 6 * * *"]} if method == "PUT" else None,
        )
        assert refused.status_code == 503, refused.text
        detail = refused.parse(ProblemDocument).detail
        assert "schedule was not changed" in detail
        assert DISABLED in detail
        current = client.get(schedule_path)
        assert current.status_code == 200, current.text
        schedule = current.parse(ScheduleResponse)
        assert current.headers["etag"] == initial_response.headers["etag"]
        assert schedule.revision == initial.revision
        assert schedule.requested == initial.requested
        stale = client.request(
            method,
            schedule_path,
            headers={"If-Match": f'"{initial.revision + 1}"'},
            json_body={"crons": ["0 9 * * *"]} if method == "PUT" else None,
        )
        assert stale.status_code == 412, stale.text
        assert stale.headers["etag"] == initial_response.headers["etag"]
        assert stale.parse(ProblemDocument).context["revision"] == initial.revision


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
@pytest.mark.parametrize("cron", ["not a cron"])
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
@pytest.mark.reliability
@pytest.mark.parametrize("method", ["GET", "PUT", "DELETE"])
def test_invalid_instance_names_are_rejected(
    admin_operator_session: PersonaSession, method: str
) -> None:
    response = admin_operator_session.client.request(
        method,
        connectors_path("/v1/connectors/INVALID/main/schedule"),
        headers={"If-Match": '"0"'},
        json_body={"crons": ["0 6 * * *"]} if method == "PUT" else None,
    )
    assert response.status_code == 400, response.text
    assert response.parse(ProblemDocument).status == 400
