from __future__ import annotations

import re
from collections.abc import Callable
from dataclasses import dataclass
from typing import Any

import pytest
from insight_stand import (
    ADMIN_OPERATOR_FIXTURE,
    SESSION_COOKIE_NAME,
    ApiClient,
    ApiResponse,
    JsonValue,
    Manifest,
    PersonaSession,
    analytics_path,
    open_session,
    v3_path,
)
from pydantic import BaseModel

from .analytics import query_window
from .identity.test_admin import _admin_role_id
from .identity.test_scenario_reach import visibility_grant
from .identity.test_scenario_role_grant import GRANTEE, held_roles, role_grant
from .schemas import (
    FeedbackListResponse,
    IngestionIntensityResponse,
    MetricResultsResponse,
    ProblemDocument,
    ReportPreviewResponse,
    UsageSummaryResponse,
)
from .schemas.analytics import MetricDrilldownResponse
from .schemas.v3_untyped import MetricEntry, Page

METRIC = "git.commits"

ANALYTICS_ADMIN_READS: dict[str, type[BaseModel]] = {
    analytics_path("/v1/usage/summary"): UsageSummaryResponse,
    analytics_path("/v1/feedback"): FeedbackListResponse,
    analytics_path("/v1/ingestion/intensity"): IngestionIntensityResponse,
}

V3_METRICS = v3_path("/v1/metrics")
ACTIVE_ROLES = "/internal/persons/active-roles"


class _AuthSession(BaseModel):
    user: str
    roles: list[str]
    csrf_token: str


class _Refreshed(BaseModel):
    expires_at: int
    refresh_at: int


class _ActiveRoles(BaseModel):
    person_id: str
    roles: list[str]


@dataclass
class _SessionCookie:
    value: str

    def headers(self) -> dict[str, str]:
        return {"Cookie": f"{SESSION_COOKIE_NAME}={self.value}"}


@dataclass(frozen=True)
class _NumbersRead:
    path: str
    body: dict[str, JsonValue]
    model: type[BaseModel]
    seen: Callable[[Any], list[Any]]


def _numbers_reads(manifest: Manifest, person_id: str) -> dict[str, _NumbersRead]:
    start, end = query_window(manifest)
    period: dict[str, JsonValue] = {"from": start, "to": end}

    return {
        "metric-results": _NumbersRead(
            analytics_path("/v1/metric-results"),
            {
                "entity": {"type": "person", "ids": [person_id]},
                "period": period,
                "metrics": [{"metric_key": METRIC, "views": [{"view": "period"}]}],
            },
            MetricResultsResponse,
            lambda parsed: [
                (value.entity_id, value.value)
                for metric in parsed.metrics
                for view in metric.root.views
                for value in view.root.values
            ],
        ),
        "report preview": _NumbersRead(
            analytics_path("/v1/reports/preview"),
            {
                "subject": {"type": "people", "ids": [person_id]},
                "period": period,
                "granularity": "month",
                "metric_keys": [METRIC],
            },
            ReportPreviewResponse,
            lambda parsed: parsed.rows,
        ),
        "drilldown": _NumbersRead(
            analytics_path("/v1/metric-drilldown"),
            {
                "metric_key": METRIC,
                "entity": {"type": "person", "id": person_id},
                "period": period,
                "filters": [],
                "display_dimensions": [],
                "limit": 50,
            },
            MetricDrilldownResponse,
            lambda parsed: parsed.rows,
        ),
    }


def _seen(client: ApiClient, reads: dict[str, _NumbersRead]) -> dict[str, list[Any]]:
    seen: dict[str, list[Any]] = {}
    for name, read in reads.items():
        response = client.post(read.path, json_body=read.body)
        assert response.status_code == 200, f"{name}: {response.status_code} {response.text[:300]}"
        seen[name] = read.seen(response.parse(read.model))
    return seen


def _refused(response: ApiResponse) -> int:
    if response.status_code != 200:
        assert response.parse(ProblemDocument).status == response.status_code
    return response.status_code


def _numbers_statuses(client: ApiClient, reads: dict[str, _NumbersRead]) -> dict[str, int]:
    return {
        name: _refused(client.post(read.path, json_body=read.body)) for name, read in reads.items()
    }


def _admin_reads(client: ApiClient) -> dict[str, int]:
    statuses: dict[str, int] = {}
    for path, model in ANALYTICS_ADMIN_READS.items():
        response = client.get(path)
        statuses[path] = _refused(response)
        if response.status_code == 200:
            response.parse(model)
    return statuses


def _auth_session(client: ApiClient) -> _AuthSession:
    response = client.get("/auth/me")
    assert response.status_code == 200, f"/auth/me: {response.status_code} {response.text[:300]}"
    return response.parse(_AuthSession)


def _refresh(client: ApiClient, cookie: _SessionCookie) -> None:
    csrf = _auth_session(client).csrf_token
    response = client.post("/auth/refresh", headers={"X-CSRF-Token": csrf})
    assert response.status_code == 200, f"refresh: {response.status_code} {response.text[:300]}"
    response.parse(_Refreshed)

    rotated = re.search(rf"{SESSION_COOKIE_NAME}=([^;]+)", response.headers.get("set-cookie", ""))
    assert rotated, f"refresh set no session cookie: {response.headers}"
    cookie.value = rotated.group(1)


def _active_roles(service_client: ApiClient, person_id: str) -> list[str]:
    response = service_client.get(ACTIVE_ROLES, params={"person_id": person_id})
    assert response.status_code == 200, f"active-roles: {response.status_code} {response.text}"
    answer = response.parse(_ActiveRoles)
    assert answer.person_id == person_id
    return answer.roles


@pytest.mark.requires_seed(ADMIN_OPERATOR_FIXTURE, "hr_lead", "dev_lead", "development_ic")
@pytest.mark.security
def test_a_visibility_grant_opens_a_persons_numbers_to_a_lead_until_revoked(
    admin_operator_session: PersonaSession,
    session_for: Callable[[str], PersonaSession],
    stand_manifest: Manifest,
) -> None:
    grantee = session_for("hr_lead")
    reference = session_for("dev_lead")
    subject = stand_manifest.fixture("development_ic")
    reads = _numbers_reads(stand_manifest, subject.uuid)
    refused = dict.fromkeys(reads, 403)

    expected = _seen(reference.client, reads)
    assert all(expected.values()), f"the IC's own lead sees no numbers to compare: {expected}"
    assert all(value is not None for _, value in expected["metric-results"])
    assert _numbers_statuses(grantee.client, reads) == refused

    with visibility_grant(admin_operator_session.client, grantee.person.uuid, subject.uuid):
        granted = _seen(grantee.client, reads)

    assert granted == expected, "the grantee saw different numbers than the IC's own lead"
    assert _numbers_statuses(grantee.client, reads) == refused


@pytest.mark.requires_seed(ADMIN_OPERATOR_FIXTURE, GRANTEE)
@pytest.mark.security
def test_granting_admin_opens_the_identity_and_analytics_admin_gates_until_revoked(
    admin_operator_session: PersonaSession,
    session_for: Callable[[str], PersonaSession],
) -> None:
    operator = admin_operator_session.client
    grantee = session_for(GRANTEE)
    admin = (_admin_role_id(operator), "admin")

    assert admin not in held_roles(grantee.client)
    assert _admin_reads(grantee.client) == dict.fromkeys(ANALYTICS_ADMIN_READS, 403)

    with role_grant(operator, grantee.person.uuid, admin[0]):
        granted_roles = held_roles(grantee.client)
        granted_reads = _admin_reads(grantee.client)

    assert admin in granted_roles
    assert granted_reads == dict.fromkeys(ANALYTICS_ADMIN_READS, 200)
    assert admin not in held_roles(grantee.client)
    assert _admin_reads(grantee.client) == dict.fromkeys(ANALYTICS_ADMIN_READS, 403)


@pytest.mark.requires_insight_v3_core
@pytest.mark.requires_seed(ADMIN_OPERATOR_FIXTURE, GRANTEE)
@pytest.mark.security
def test_granting_admin_opens_v3_authoring_until_revoked(
    admin_operator_session: PersonaSession,
    session_for: Callable[[str], PersonaSession],
) -> None:
    operator = admin_operator_session.client
    grantee = session_for(GRANTEE)

    assert _refused(grantee.client.get(V3_METRICS)) == 403

    with role_grant(operator, grantee.person.uuid, _admin_role_id(operator)):
        granted = grantee.client.get(V3_METRICS)
        assert granted.status_code == 200, f"v3 metrics: {granted.status_code} {granted.text}"
        granted.parse(Page[MetricEntry])

    assert _refused(grantee.client.get(V3_METRICS)) == 403


@pytest.mark.requires_seed(ADMIN_OPERATOR_FIXTURE, GRANTEE)
@pytest.mark.security
def test_a_session_takes_on_a_granted_admin_role_at_its_next_refresh(
    admin_operator_session: PersonaSession,
    stand_manifest: Manifest,
    stand_base_url: str,
) -> None:
    operator = admin_operator_session.client
    login = open_session(GRANTEE, stand_manifest, stand_base_url)
    cookie = _SessionCookie(login.session.headers()["Cookie"].split("=", 1)[1])
    browser = ApiClient(base_url=stand_base_url, session=cookie)
    try:
        before = _auth_session(browser)
        assert before.user == login.person.uuid
        assert "admin" not in before.roles

        with role_grant(operator, login.person.uuid, _admin_role_id(operator)):
            assert "admin" not in _auth_session(browser).roles, (
                "the session changed before a refresh"
            )
            _refresh(browser, cookie)
            granted = _auth_session(browser).roles

        assert "admin" in granted, f"a refresh during the grant left the session with {granted}"

        _refresh(browser, cookie)
        assert "admin" not in _auth_session(browser).roles, "a refresh after revoke kept admin"
    finally:
        csrf = _auth_session(browser).csrf_token
        browser.post("/auth/logout", headers={"X-CSRF-Token": csrf})


@pytest.mark.requires_service_principal
@pytest.mark.requires_seed(ADMIN_OPERATOR_FIXTURE, GRANTEE)
@pytest.mark.security
def test_the_service_role_read_behind_a_refresh_follows_the_grant(
    admin_operator_session: PersonaSession,
    session_for: Callable[[str], PersonaSession],
    service_client: ApiClient,
) -> None:
    operator = admin_operator_session.client
    grantee = session_for(GRANTEE).person.uuid

    assert "admin" not in _active_roles(service_client, grantee)

    with role_grant(operator, grantee, _admin_role_id(operator)):
        granted = _active_roles(service_client, grantee)

    assert "admin" in granted
    assert "admin" not in _active_roles(service_client, grantee)
