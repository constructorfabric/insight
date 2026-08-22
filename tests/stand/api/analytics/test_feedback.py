"""The `/v1/feedback` pair on analytics — product feedback in and back out.

    POST /v1/feedback   204 · any signed-in caller · 400 empty message
    GET  /v1/feedback   200 for the admin operator · 400 malformed day ·
                        403 for everybody else

One url, two audiences. Sending is open because the dialog is in the sidebar of
every screen; reading is admin-only, and the gate is inside the handler rather
than at the edge — it asks identity `/v1/me` for an active `admin` row, the same
grant `/v1/usage/summary` reads. So the gateway sees two ordinary authenticated
routes and the refusal is only observable with a session.

Feedback is the second place this suite writes rows it cannot remove: no
operation deletes a submission, so `scratch.py`'s create-then-delete policy does
not reach them. Every message this module sends carries `SCRATCH_PREFIX` and the
run tag so a row left on the stand is attributable, and the readback asks for
this run's own message rather than for a position in the listing — the endpoint
serves the newest 200 of a window, and a stand that has collected submissions
from earlier runs would push a fresh one down a fixed-size page.

The 401 half is in `test_gateway.py`, swept over every operation at once, and
the 415 half is in `test_request_contracts.py` with every other body route.
"""

from __future__ import annotations

from datetime import UTC, datetime

import pytest
from insight_stand import ApiClient, PersonaSession, analytics_path

from .. import scratch
from ..schemas import FeedbackListResponse, ProblemDocument

FEEDBACK = analytics_path("/v1/feedback")

#: The screen the sender was on, in the shape the SPA sends: the person the page
#: is about is already reduced to `:id` before it leaves the browser.
SENT_FROM = "/ic/:id/personal"


def _message() -> str:
    return f"{scratch.SCRATCH_PREFIX}-{scratch.RUN_TAG} product feedback from the stand suite"


def _today() -> str:
    """The handler buckets by whole UTC days, so the run's window is that date."""
    return datetime.now(UTC).date().isoformat()


def _listing(client: ApiClient, since: str) -> FeedbackListResponse:
    # `since` is captured before the write and `until` read now, so a run that
    # straddles UTC midnight still spans the day its own submission landed on.
    response = client.get(FEEDBACK, params={"since": since, "until": _today()})
    assert response.status_code == 200, f"listing: {response.status_code} {response.text[:300]}"
    return response.parse(FeedbackListResponse)


@pytest.fixture(scope="module")
def listing_after_a_submission(
    lead_session: PersonaSession, admin_operator_session: PersonaSession
) -> tuple[str, FeedbackListResponse]:
    """Send one submission as an ordinary caller, then read it back as admin.

    Module-scoped so the run adds one undeletable row rather than one per test,
    and so both assertions below describe the same submission instead of racing
    each other to write one.
    """
    message = _message()
    day = _today()

    accepted = lead_session.client.post(FEEDBACK, json_body={"message": message, "path": SENT_FROM})
    assert accepted.status_code == 204, (
        f"submitting answered {accepted.status_code}, expected 204: {accepted.text[:300]}"
    )

    return message, _listing(admin_operator_session.client, day)


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_a_submission_is_recorded_and_reaches_the_listing(
    listing_after_a_submission: tuple[str, FeedbackListResponse],
) -> None:
    """The whole point of the feature, end to end on the deployed path.

    A 204 proves the request was accepted and nothing else — the write is the
    only thing the caller cannot see, so the reader is what shows it happened.
    The screen the sender was on travels with the message: without it a report
    says something is broken and never says where.
    """
    message, listing = listing_after_a_submission
    entries = {entry.message: entry for entry in listing.items}
    assert message in entries, (
        f"the submission is absent from the listing, which carries {len(listing.items)} entries "
        f"for {listing.since}..{listing.until}"
    )
    assert entries[message].path == SENT_FROM
    assert entries[message].person_id, "the entry names no sender"


@pytest.mark.reliability
def test_an_empty_message_is_refused_rather_than_stored(api: ApiClient) -> None:
    """A blank submission is a row nobody can act on and nobody can remove.

    Whitespace, not `""`: an empty string is refused by any check, and the one
    that matters is the trim — the dialog sends whatever the box holds.
    """
    response = api.post(FEEDBACK, json_body={"message": "   ", "path": SENT_FROM})
    assert response.status_code == 400, (
        f"a blank message answered {response.status_code}: {response.text[:300]}"
    )
    assert response.parse(ProblemDocument).status == 400


@pytest.mark.security
def test_the_listing_is_refused_without_the_admin_grant(api: ApiClient) -> None:
    """Feedback carries whatever the sender typed, addressed to the operator.

    The SPA shows the page to admins only, which is a courtesy and not a
    boundary: anybody signed in can address the url, and what they would read is
    other people's messages. This is the assertion that the boundary exists.
    """
    response = api.get(FEEDBACK)
    assert response.status_code == 403, (
        f"the feedback listing answered {response.status_code} to a caller holding no "
        f"admin grant: {response.text[:300]}"
    )
    problem = response.parse(ProblemDocument)
    assert problem.status == 403
    assert problem.detail, "the refusal carries no detail a caller can act on"


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_a_malformed_day_is_refused_rather_than_queried(
    admin_operator_session: PersonaSession,
) -> None:
    """The window is parsed before the database is asked anything.

    `since` and `until` are caller-supplied and reach a query, so the failure to
    avoid is one where an unparseable value is carried far enough to become a
    500 — or worse, part of a statement.
    """
    response = admin_operator_session.client.get(FEEDBACK, params={"since": "not-a-date"})
    assert response.status_code == 400, (
        f"a malformed `since` answered {response.status_code}: {response.text[:300]}"
    )
    assert response.parse(ProblemDocument).status == 400
