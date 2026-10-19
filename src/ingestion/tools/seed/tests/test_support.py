"""The support generator emits events that count back to the volumes it drew.

`zendesk__support_activity` derives every support measure from audits and
ratings, so the generator no longer states the numbers — it states the events
they are counted from. These tests apply the staging model's own
classification rules in Python, which is the cheap half of the contract; the
expensive half (that dbt agrees) needs a warehouse and lives in the data-path
suite.
"""

from __future__ import annotations

import datetime as _dt
import json

import pytest

from insight_seed.generators import support
from insight_seed.generators.base import days_window
from insight_seed.profiles import build_roster

DEV_EMAIL = "dev@company.nonpresent"
ANCHOR = _dt.date(2026, 10, 1)


@pytest.fixture(scope="module")
def support_people() -> list:
    return support._eligible(build_roster(DEV_EMAIL))


def _classify(event: dict) -> str:
    """`zendesk__support_event`'s multiIf, in Python."""
    if event["type"] == "Comment":
        return "public_comment" if event["public"] else "private_comment"
    if event.get("field_name") == "status" and event.get("value") == "solved":
        return "solved"
    return "update"


def test_the_roster_supplies_support_people(support_people: list) -> None:
    """Every assertion below is vacuous without them."""
    assert support_people, "no person carries a support weight, so nothing is emitted"


def test_the_emitted_audits_count_back_to_the_drawn_volumes(support_people: list) -> None:
    """What silver serves is a count over these events, so the counts must agree.

    `updates` counts DISTINCT audits and `solved` DISTINCT tickets — the
    comment measures count events — which is why each is asserted its own way.
    """
    for person in support_people:
        for day in days_window(14, end=ANCHOR + _dt.timedelta(days=1)):
            volumes = support._volumes(person, day)
            audits = list(support._audits(person, day, volumes))

            update_audits, solved_tickets = set(), set()
            comments = {"public_comment": 0, "private_comment": 0}
            for audit in audits:
                for event in json.loads(audit.events):
                    kind = _classify(event)
                    if kind == "update":
                        update_audits.add(audit.audit_id)
                    elif kind == "solved":
                        solved_tickets.add(audit.ticket_id)
                    else:
                        comments[kind] += 1

            case = f"{person.email} on {day}"
            assert len(update_audits) == volumes.updates, f"updates disagree for {case}"
            assert len(solved_tickets) == volumes.solved, f"solved tickets disagree for {case}"
            assert comments["public_comment"] == volumes.public_comments, (
                f"public comments disagree for {case}"
            )
            assert comments["private_comment"] == volumes.private_comments, (
                f"private comments disagree for {case}"
            )


def test_every_audit_is_uniquely_identified(support_people: list) -> None:
    """Bronze dedups on unique_key and the event key is (audit, event id), so a
    repeat would silently drop a day's work rather than duplicate it."""
    seen: set[str] = set()
    for person in support_people:
        for day in days_window(14, end=ANCHOR + _dt.timedelta(days=1)):
            for audit in support._audits(person, day, support._volumes(person, day)):
                assert audit.audit_id not in seen, f"audit {audit.audit_id} is emitted twice"
                seen.add(audit.audit_id)


def test_the_draw_order_is_fixed() -> None:
    """`poisson` consumes rng state, so reordering the draws reseeds every
    support row in the dataset. These are the values the committed order gives."""
    person = next(p for p in build_roster(DEV_EMAIL) if p.team == "support" and p.role == "lead")

    volumes = support._volumes(person, _dt.date(2026, 9, 30))

    assert tuple(volumes) == (3, 3, 1, 0, 0, 0), (
        f"the support draws moved: {tuple(volumes)}. If that was deliberate, every "
        f"stand's support numbers changed with it."
    )
