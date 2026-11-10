from __future__ import annotations

from collections.abc import Callable
from datetime import UTC, datetime

import pytest
from insight_stand import ApiClient, PersonaSession, analytics_path

from .. import scratch
from ..schemas import FeedbackListResponse, ProblemDocument

FEEDBACK = analytics_path("/v1/feedback")

MESSAGE_BUDGET = 4000

SENT_FROM = "/team/:id/overview"

TAG = f"{scratch.SCRATCH_PREFIX}-{scratch.RUN_TAG}-feedback"


def _today() -> str:
    return datetime.now(UTC).date().isoformat()


def _listing(client: ApiClient, since: str) -> FeedbackListResponse:
    response = client.get(FEEDBACK, params={"since": since, "until": _today()})
    assert response.status_code == 200, f"listing: {response.status_code} {response.text[:300]}"
    return response.parse(FeedbackListResponse)


def _at_budget() -> str:
    head = f"{TAG} Отзыв: экспорт не открывается 🙂\nline two\t"
    return head + "ж" * (MESSAGE_BUDGET - len(head))


def _over_budget() -> str:
    head = f"{TAG}-over "
    return head + "x" * (MESSAGE_BUDGET + 1 - len(head))


@pytest.mark.requires_seed("support_ic", "admin_operator")
@pytest.mark.reliability
def test_feedback_from_a_member_reaches_the_operator_whole_and_attributed(
    session_for: Callable[[str], PersonaSession],
    admin_operator_session: PersonaSession,
) -> None:
    sender = session_for("support_ic")
    day = _today()
    message = _at_budget()
    assert len(message) == MESSAGE_BUDGET

    accepted = sender.client.post(
        FEEDBACK, json_body={"message": f"\n  {message}  \n", "path": SENT_FROM}
    )
    assert accepted.status_code == 204, accepted.text[:300]

    blank = sender.client.post(FEEDBACK, json_body={"message": " \n\t ", "path": SENT_FROM})
    assert blank.status_code == 400, blank.text[:300]
    assert blank.parse(ProblemDocument).status == 400

    over = sender.client.post(FEEDBACK, json_body={"message": _over_budget(), "path": SENT_FROM})
    assert over.status_code == 400, over.text[:300]
    assert over.parse(ProblemDocument).status == 400

    entries = _listing(admin_operator_session.client, day).items
    ours = [entry for entry in entries if entry.message.startswith(TAG)]

    assert [entry.message for entry in ours] == [message], [e.message[:80] for e in ours]
    received = ours[0]
    assert received.person_id == sender.person.uuid
    assert received.email == sender.person.email
    assert received.display_name == sender.person.display_name
    assert received.path == SENT_FROM
    assert received.ts.startswith(day)
    assert all(entry.message.strip() for entry in entries), "a blank submission was stored"


@pytest.mark.requires_seed("support_ic")
@pytest.mark.security
def test_a_member_who_sends_feedback_cannot_read_the_listing(
    session_for: Callable[[str], PersonaSession],
) -> None:
    response = session_for("support_ic").client.get(FEEDBACK, params={"since": _today()})

    assert response.status_code == 403, response.text[:300]
    assert response.parse(ProblemDocument).status == 403
