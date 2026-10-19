"""Jira links as intervals in `class_task_links`, keyed by immutable issue ids.

The changelog logs a link added or removed as one `issuelinks` item naming the
linked issue by the key it had then, with a sentence ("This issue blocks
TST-2") that is the only place its type survives. The snapshot holds the link
object, with the linked issue's id and the type's name and phrases. These
scenarios pin how the two meet: intervals fold from the events, a link held
before its first event is seeded from the snapshot, and the key resolves to the
id on both the links and the journal.
"""

from __future__ import annotations

from typing import Any

from conftest import LINKS_SELECTOR, Scenario, case
from helpers import event, field, issue, item

SELECTOR = LINKS_SELECTOR

LINKS = "issuelinks"
LINKS_FIELD = field(LINKS, name="Linked Issues", schema_type="array", schema_items="issuelinks")
PARENT = "parent"
PARENT_FIELD = field(PARENT, name="Parent")

BLOCKS = {"id": "10000", "name": "Blocks", "outward": "blocks", "inward": "is blocked by"}
DUPLICATE = {"id": "10001", "name": "Duplicate", "outward": "duplicates", "inward": "is duplicated by"}
CAUSES = {"id": "10900", "name": "Causes", "outward": "causes", "inward": "is caused by"}
DEPENDENCE = {"id": "10610", "name": "Dependence", "outward": "blocks (Dependence)", "inward": "depends on (blocks)"}

ISSUE_WORDING = "This issue"
WORK_ITEM_WORDING = "This work item"

CREATED = "2026-01-05 09:00:00.000"
DAY_6 = "2026-01-06 10:00:00.000"
DAY_7 = "2026-01-07 10:00:00.000"
DAY_8 = "2026-01-08 10:00:00.000"


def _outward(link_id: str, link_type: dict[str, str], issue_id: str, key: str) -> dict[str, Any]:
    return {"id": link_id, "type": link_type, "outwardIssue": {"id": issue_id, "key": key}}


def _inward(link_id: str, link_type: dict[str, str], issue_id: str, key: str) -> dict[str, Any]:
    return {"id": link_id, "type": link_type, "inwardIssue": {"id": issue_id, "key": key}}


def _added(key: str, relation: str, wording: str = ISSUE_WORDING) -> dict[str, Any]:
    return item(LINKS, to=key, to_str=f"{wording} {relation} {key}")


def _removed(key: str, relation: str, wording: str = ISSUE_WORDING) -> dict[str, Any]:
    return item(LINKS, frm=key, frm_str=f"{wording} {relation} {key}")


def _intervals(rows: list[dict[str, Any]]) -> list[tuple[Any, ...]]:
    return [
        (
            r["issue_id"],
            r["link_type"],
            r["target_id"],
            r["target_readable"],
            r["valid_from"],
            r["valid_from_known"],
            r["valid_to"],
            r["origin_event_id"],
        )
        for r in rows
    ]


@case(
    fields=[LINKS_FIELD],
    issues=[
        issue("TST-1", jira_id="1001", fields={LINKS: [_outward("1", BLOCKS, "1002", "TST-2")]}),
        issue("TST-2", jira_id="1002", fields={LINKS: [_inward("1", BLOCKS, "1001", "TST-1")]}),
    ],
    events=[
        event("TST-1", 101, "2026-01-06T10:00:00", [_added("TST-2", "blocks")], jira_id="1001"),
        event("TST-1", 102, "2026-01-07T10:00:00", [_removed("TST-2", "blocks")], jira_id="1001"),
        event("TST-1", 103, "2026-01-08T10:00:00", [_added("TST-2", "blocks")], jira_id="1001"),
        event("TST-2", 201, "2026-01-06T10:00:00", [_added("TST-1", "is blocked by")], jira_id="1002"),
        event("TST-2", 202, "2026-01-07T10:00:00", [_removed("TST-1", "is blocked by")], jira_id="1002"),
        event("TST-2", 203, "2026-01-08T10:00:00", [_added("TST-1", "is blocked by")], jira_id="1002"),
    ],
)
def test_a_link_removed_and_added_again_is_two_intervals_on_each_side(scenario: Scenario) -> None:
    """Jira logs a link on both issues, each in its own words, so each side
    folds its own events; the second addition opens a second occurrence."""
    assert _intervals(scenario.links()) == [
        ("1001", "blocking", "1002", "TST-2", DAY_6, 1, DAY_7, "101"),
        ("1001", "blocking", "1002", "TST-2", DAY_8, 1, None, "103"),
        ("1002", "blocked_by", "1001", "TST-1", DAY_6, 1, DAY_7, "201"),
        ("1002", "blocked_by", "1001", "TST-1", DAY_8, 1, None, "203"),
    ]
    closed = scenario.links(issue_id="1001")[0]
    assert (closed["added_by"], closed["removed_by"], closed["evidence"]) == ("author-acct", "author-acct", "event")
    assert closed["is_cross_repository"] is False
    assert scenario.states(LINKS, issue="TST-1") == [[], ["1002"], [], ["1002"]]
    assert scenario.round_trip_holds()


@case(
    fields=[LINKS_FIELD],
    issues=[
        issue("TST-1", jira_id="1001", fields={LINKS: [_outward("2", DUPLICATE, "1002", "TST-2")]}),
        issue("TST-2", jira_id="1002"),
    ],
    events=[
        event("TST-1", 101, "2026-01-06T10:00:00", [_added("TST-2", "relates to")], jira_id="1001"),
        event("TST-1", 102, "2026-01-07T10:00:00", [_added("TST-2", "duplicates")], jira_id="1001"),
        event("TST-1", 103, "2026-01-08T10:00:00", [_removed("TST-2", "relates to")], jira_id="1001"),
    ],
)
def test_a_second_link_to_one_issue_is_its_own_occurrence(scenario: Scenario) -> None:
    """Two links to one issue are one element of `issuelinks`, which counts
    them, and two occurrences here: removing one closes only its own."""
    assert _intervals(scenario.links(issue_id="1001")) == [
        ("1001", "connected", "1002", "TST-2", DAY_6, 1, DAY_8, "101"),
        ("1001", "duplicate_of", "1002", "TST-2", DAY_7, 1, None, "102"),
    ]
    assert scenario.states(LINKS, issue="TST-1") == [[], ["1002"], ["1002"], ["1002"]]
    assert scenario.round_trip_holds()


@case(
    fields=[LINKS_FIELD],
    issues=[issue("TST-1", jira_id="1001"), issue("NEW-9", jira_id="7005")],
    events=[
        event("TST-1", 101, "2026-01-06T10:00:00", [_added("OLD-5", "blocks")], jira_id="1001"),
        event(
            "NEW-9",
            150,
            "2026-01-07T10:00:00",
            [
                {
                    "field": "Key",
                    "fieldtype": "jira",
                    "from": None,
                    "fromString": "OLD-5",
                    "to": None,
                    "toString": "NEW-9",
                }
            ],
            jira_id="7005",
        ),
        event("TST-1", 102, "2026-01-08T10:00:00", [_removed("NEW-9", "blocks")], jira_id="1001"),
    ],
)
def test_a_link_to_an_issue_that_moved_is_one_interval(scenario: Scenario) -> None:
    """The addition names the linked issue by its old key and the removal by
    its new one. Bronze holds no row under the old key; only the moved issue's
    own `Key` event ties the two to one id."""
    assert _intervals(scenario.links()) == [("1001", "blocking", "7005", "NEW-9", DAY_6, 1, DAY_8, "101")]
    assert scenario.changelog_states(LINKS, issue="TST-1") == [["7005"], []]
    assert scenario.round_trip_holds()


@case(
    fields=[PARENT_FIELD],
    issues=[
        issue("TST-1", jira_id="1001"),
        issue("TST-2", jira_id="1002"),
        issue("TST-3", jira_id="1003", fields={PARENT: {"id": "1002", "key": "TST-2"}}),
    ],
    events=[
        event(
            "TST-3",
            101,
            "2026-01-06T10:00:00",
            [item(PARENT, frm="1001", frm_str="TST-1", to="1002", to_str="TST-2")],
            jira_id="1003",
        )
    ],
)
def test_a_parent_change_closes_one_parent_and_opens_the_next_on_both_sides(scenario: Scenario) -> None:
    """Jira logs a parent change on the child alone, so the parents' `sub_issue`
    side is mirrored from it. The first parent was there before any event."""
    assert _intervals(scenario.links()) == [
        ("1001", "sub_issue", "1003", "TST-3", CREATED, 0, DAY_6, None),
        ("1002", "sub_issue", "1003", "TST-3", DAY_6, 1, None, "101"),
        ("1003", "parent", "1001", "TST-1", CREATED, 0, DAY_6, None),
        ("1003", "parent", "1002", "TST-2", DAY_6, 1, None, "101"),
    ]
    assert scenario.states(PARENT, issue="TST-3") == [["1001"], ["1002"]]
    assert scenario.round_trip_holds()


@case(
    fields=[LINKS_FIELD],
    issues=[
        issue("TST-1", jira_id="1001", fields={LINKS: [_inward("9", CAUSES, "1002", "TST-2")]}),
        issue("TST-2", jira_id="1002"),
        issue("TST-4", jira_id="1004"),
    ],
    events=[
        event("TST-1", 101, "2026-01-06T10:00:00", [_added("TST-2", "is caused by")], jira_id="1001"),
        event("TST-1", 102, "2026-01-07T10:00:00", [_removed("TST-4", "is blocked by")], jira_id="1001"),
    ],
)
def test_a_custom_link_type_is_connected_and_an_unseen_type_reads_its_wording(scenario: Scenario) -> None:
    """A type outside the contract's vocabulary is `connected`. A relation no
    snapshot names a type for falls back to Jira's default wording, and a link
    whose first event removed it was held from the issue's creation."""
    assert _intervals(scenario.links()) == [
        ("1001", "blocked_by", "1004", "TST-4", CREATED, 0, DAY_7, None),
        ("1001", "connected", "1002", "TST-2", DAY_6, 1, None, "101"),
    ]
    assert scenario.states(LINKS, issue="TST-1") == [["1004"], ["1004", "1002"], ["1002"]]
    assert scenario.round_trip_holds()


@case(
    fields=[LINKS_FIELD],
    issues=[issue("TST-1", jira_id="1001", fields={LINKS: [_outward("3", BLOCKS, "1002", "TST-2")]})],
)
def test_a_link_no_event_reaches_opens_at_creation_as_a_bound(scenario: Scenario) -> None:
    """The linked issue is not synced: the snapshot alone names its id."""
    rows = scenario.links()
    assert _intervals(rows) == [("1001", "blocking", "1002", "TST-2", CREATED, 0, None, None)]
    assert (rows[0]["added_by"], rows[0]["evidence"]) == (None, "event")
    assert scenario.states(LINKS, issue="TST-1") == [["1002"]]
    assert scenario.round_trip_holds()


@case(
    fields=[LINKS_FIELD],
    issues=[issue("TST-1", jira_id="1001")],
    events=[
        event("TST-1", 101, "2026-01-06T10:00:00", [_added("EXT-4", "relates to")], jira_id="1001"),
        event("TST-1", 102, "2026-01-07T10:00:00", [_removed("EXT-4", "relates to")], jira_id="1001"),
        event(
            "TST-1",
            103,
            "2026-01-08T10:00:00",
            [item(LINKS, to="70001", to_str='This issue links to "Page"')],
            jira_id="1001",
        ),
    ],
)
def test_a_key_that_resolves_to_no_id_leaves_the_target_id_empty(scenario: Scenario) -> None:
    """Nothing synced names EXT-4's id. The key still keeps the occurrence
    apart and labels it, but is never put where the id belongs. A remote link
    is no issue link at all."""
    rows = scenario.links()
    assert _intervals(rows) == [("1001", "connected", None, "EXT-4", DAY_6, 1, DAY_7, "101")]
    assert rows[0]["is_cross_repository"] is True
    assert scenario.changelog_states(LINKS, issue="TST-1") == [["EXT-4"], []]
    assert scenario.round_trip_holds()


@case(
    fields=[LINKS_FIELD],
    issues=[
        issue("TST-1", jira_id="1001", fields={LINKS: [_outward("1", BLOCKS, "1002", "TST-2")]}),
        issue("TST-2", jira_id="1002"),
    ],
    events=[
        event("TST-1", 101, "2026-01-06T10:00:00", [_added("TST-2", "blocks")], jira_id="1001"),
        event("TST-1", 102, "2026-01-07T10:00:00", [_removed("TST-2", "blocks", WORK_ITEM_WORDING)], jira_id="1001"),
        event("TST-1", 103, "2026-01-08T10:00:00", [_added("TST-2", "blocks", WORK_ITEM_WORDING)], jira_id="1001"),
    ],
)
def test_a_link_reads_the_same_whichever_subject_the_sentence_names(scenario: Scenario) -> None:
    """Jira opens the sentence with "This issue" or "This work item"; the
    subject says nothing about the link, so one link stays one occurrence."""
    assert _intervals(scenario.links()) == [
        ("1001", "blocking", "1002", "TST-2", DAY_6, 1, DAY_7, "101"),
        ("1001", "blocking", "1002", "TST-2", DAY_8, 1, None, "103"),
    ]
    assert scenario.round_trip_holds()


@case(
    fields=[LINKS_FIELD],
    issues=[
        issue("TST-1", jira_id="1001", fields={LINKS: [_outward("1", DEPENDENCE, "1002", "TST-2")]}),
        issue("TST-2", jira_id="1002", fields={LINKS: [_inward("1", DEPENDENCE, "1001", "TST-1")]}),
        issue("TST-3", jira_id="1003", fields={LINKS: [_outward("2", BLOCKS, "1004", "TST-4")]}),
        issue("TST-4", jira_id="1004"),
    ],
    events=[
        event("TST-1", 101, "2026-01-06T10:00:00", [_added("TST-2", "blocks")], jira_id="1001"),
        event("TST-2", 201, "2026-01-06T10:00:00", [_added("TST-1", "depends on")], jira_id="1002"),
        event("TST-3", 301, "2026-01-06T10:00:00", [_added("TST-4", "blocks")], jira_id="1003"),
    ],
)
def test_a_reworded_link_type_is_the_type_the_link_holds_not_the_phrase_it_had(scenario: Scenario) -> None:
    """The changelog keeps the phrase of its day. A Dependence link added while
    the type still read "blocks" is the Dependence link the snapshot holds
    between the same two issues, not a Blocks link that now owns that phrase."""
    assert _intervals(scenario.links()) == [
        ("1001", "connected", "1002", "TST-2", DAY_6, 1, None, "101"),
        ("1002", "connected", "1001", "TST-1", DAY_6, 1, None, "201"),
        ("1003", "blocking", "1004", "TST-4", DAY_6, 1, None, "301"),
    ]
    assert scenario.round_trip_holds()


@case(
    fields=[LINKS_FIELD],
    issues=[
        issue("TST-1", jira_id="1001", fields={LINKS: [_outward("1", BLOCKS, "1002", "TST-2")]}),
        issue("TST-2", jira_id="1002"),
        issue("TST-5", jira_id="1005"),
        issue("TST-6", jira_id="1006"),
    ],
    events=[
        event("TST-1", 101, "2026-01-06T10:00:00", [_added("TST-2", "must finish before")], jira_id="1001"),
        event("TST-5", 501, "2026-01-06T10:00:00", [_added("TST-6", "must finish before")], jira_id="1005"),
        event("TST-5", 502, "2026-01-07T10:00:00", [_removed("TST-6", "blocks", WORK_ITEM_WORDING)], jira_id="1005"),
    ],
)
def test_a_retired_phrase_means_the_type_it_was_seen_on_for_a_link_no_longer_held(scenario: Scenario) -> None:
    """No snapshot holds TST-5's link any more. Its old phrase was seen on a
    Blocks link that is still held, so the addition in the old phrase and the
    removal in the new one are the same Blocks link."""
    assert _intervals(scenario.links()) == [
        ("1001", "blocking", "1002", "TST-2", DAY_6, 1, None, "101"),
        ("1005", "blocking", "1006", "TST-6", DAY_6, 1, DAY_7, "501"),
    ]
    assert scenario.round_trip_holds()


@case(
    fields=[LINKS_FIELD],
    issues=[
        issue("TST-1", jira_id="1001", fields={LINKS: [_outward("1", BLOCKS, "1002", "TST-2")]}),
        issue("TST-2", jira_id="1002"),
    ],
    events=[
        event("TST-1", 101, "2026-01-06T10:00:00", [_added("TST-2", "must finish before")], jira_id="1001"),
        event("TST-1", 102, "2026-01-07T10:00:00", [_removed("TST-2", "blocks")], jira_id="1001"),
        event("TST-1", 103, "2026-01-08T10:00:00", [_added("TST-2", "blocks")], jira_id="1001"),
    ],
)
def test_a_link_re_added_after_its_type_was_reworded_is_one_occurrence(scenario: Scenario) -> None:
    """The new phrase states the held link but first removes it, so the link
    was there before under another phrase: the old phrase still added is it."""
    assert _intervals(scenario.links()) == [
        ("1001", "blocking", "1002", "TST-2", DAY_6, 1, DAY_7, "101"),
        ("1001", "blocking", "1002", "TST-2", DAY_8, 1, None, "103"),
    ]
    assert scenario.round_trip_holds()
