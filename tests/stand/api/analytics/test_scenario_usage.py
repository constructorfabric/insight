from __future__ import annotations

import json
from collections.abc import Callable
from datetime import UTC, datetime

import pytest
from insight_stand import ApiClient, PersonaSession, analytics_path, wait_until

from .. import scratch
from ..schemas import (
    ProblemDocument,
    UsageActionsResponse,
    UsageConfigResponse,
    UsagePage,
    UsagePagesResponse,
    UsagePeopleResponse,
    UsagePerson,
    UsageSummaryResponse,
)

EVENTS = analytics_path("/v1/usage/events")
CONFIG = analytics_path("/v1/usage/config")
SUMMARY = analytics_path("/v1/usage/summary")
PEOPLE = analytics_path("/v1/usage/people")
PAGES = analytics_path("/v1/usage/pages")
ACTIONS = analytics_path("/v1/usage/actions")

SDK_CONTENT_TYPE = "application/vnd.kafka.json.v2+json"

TAG = f"{scratch.SCRATCH_PREFIX}-{scratch.RUN_TAG}-visit"
VISITED = f"/portal/{TAG}"
DRILLED = f"scratch.{TAG}"
SESSION_TARGET = f"{TAG}-session"


def _today() -> str:
    return datetime.now(UTC).date().isoformat()


def _read(client: ApiClient, path: str, since: str, **params: str) -> str:
    response = client.get(path, params={"since": since, "until": _today(), **params})
    assert response.status_code == 200, f"{path}: {response.status_code} {response.text[:300]}"
    return response.text


def _summary(client: ApiClient, since: str) -> UsageSummaryResponse:
    return UsageSummaryResponse.model_validate_json(_read(client, SUMMARY, since))


def _pages(client: ApiClient, since: str) -> list[UsagePage]:
    return UsagePagesResponse.model_validate_json(_read(client, PAGES, since)).items


def _people(client: ApiClient, since: str, **params: str) -> list[UsagePerson]:
    return UsagePeopleResponse.model_validate_json(_read(client, PEOPLE, since, **params)).items


def _figures(people: list[UsagePerson], person_id: str) -> tuple[int, int]:
    for person in people:
        if person.person_id == person_id:
            return person.visits, person.page_views
    return 0, 0


def _visit() -> str:
    return json.dumps(
        {
            "meta": {
                "context_session_id": TAG,
                "context_app_name": "insight-stand-tests",
                "context_app_version": "0",
            },
            "records": [
                {"name": "session_start", "data": {"target": SESSION_TARGET}},
                {"name": "page_view", "data": {"path": VISITED}},
                {"name": "drill", "data": {"target": DRILLED, "path": VISITED}},
                {"name": "page_view", "data": {"path": VISITED}},
            ],
        }
    )


@pytest.mark.requires_seed("sales_ic", "admin_operator")
@pytest.mark.reliability
def test_a_visit_by_a_member_reaches_every_operator_view_of_usage(
    session_for: Callable[[str], PersonaSession],
    admin_operator_session: PersonaSession,
) -> None:
    visitor = session_for("sales_ic")
    admin = admin_operator_session.client

    config = visitor.client.get(CONFIG)
    assert config.status_code == 200, config.text[:300]
    if not config.parse(UsageConfigResponse).enabled:
        pytest.skip("this instance does not record usage")

    day = _today()
    summary_before = _summary(admin, day)
    visits_before, views_before = _figures(_people(admin, day), visitor.person.uuid)

    accepted = visitor.client.post(
        EVENTS, content=_visit(), headers={"Content-Type": SDK_CONTENT_TYPE}
    )
    assert accepted.status_code == 204, accepted.text[:300]

    wait_until(
        lambda: VISITED in {page.path for page in _pages(admin, day)},
        timeout_s=20,
        description=f"the visit to {VISITED} to reach the pages list",
    )

    pages = {page.path: page for page in _pages(admin, day)}
    assert (pages[VISITED].views, pages[VISITED].visitors) == (2, 1)

    actions = UsageActionsResponse.model_validate_json(_read(admin, ACTIONS, day)).items
    drills = [a for a in actions if (a.event_name, a.target) == ("drill", DRILLED)]
    assert [(a.opens, a.people) for a in drills] == [(1, 1)], drills
    assert SESSION_TARGET not in {a.target for a in actions}, "session_start counted as an action"
    assert "page_view" not in {a.event_name for a in actions}, "page views counted as actions"

    people = _people(admin, day)
    assert _figures(people, visitor.person.uuid) == (visits_before + 1, views_before + 2)

    summary_after = _summary(admin, day)
    assert summary_after.totals.page_views >= summary_before.totals.page_views + 2
    assert summary_after.totals.visits >= summary_before.totals.visits + 1
    today = [d for d in summary_after.by_day if d.day == day]
    assert today and today[0].visitors >= 1, summary_after.by_day

    row = next(p for p in people if p.person_id == visitor.person.uuid)
    needle = row.display_name or row.username
    assert needle, "the visitor carries no name to search by"
    found = _people(admin, day, search=needle.upper())
    assert {p.person_id for p in found} == {visitor.person.uuid}, [p.display_name for p in found]


@pytest.mark.requires_seed("sales_ic")
@pytest.mark.security
@pytest.mark.parametrize("path", [SUMMARY, PEOPLE, PAGES, ACTIONS])
def test_a_member_who_records_usage_cannot_read_it_back(
    session_for: Callable[[str], PersonaSession], path: str
) -> None:
    response = session_for("sales_ic").client.get(path, params={"since": _today()})

    assert response.status_code == 403, f"{path}: {response.status_code} {response.text[:300]}"
    assert response.parse(ProblemDocument).status == 403
