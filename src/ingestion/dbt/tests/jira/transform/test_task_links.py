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

import pytest
from conftest import LINKS_SELECTOR, Build, Scenario, case, phased
from helpers import census_entry, event, field, issue, item, project_visibility

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


EPIC_LINK = "customfield_11300"
EPIC_LINK_FIELD = field(
    EPIC_LINK, name="Epic Link", schema_type="any", schema_custom="com.pyxis.greenhopper.jira:gh-epic-link"
)
READ = "2026-03-01 12:00:00.000"
UPDATED = "2026-02-01 09:00:00.000"


@case(
    fields=[PARENT_FIELD],
    issues=[
        issue("TST-2", jira_id="1002"),
        issue("TST-3", jira_id="1003", fields={PARENT: {"id": "1002", "key": "TST-2"}}),
    ],
    events=[
        event(
            "TST-3",
            101,
            "2026-01-06T10:00:00",
            [item(PARENT, to="1002", to_str="TST-2"), item(PARENT, frm="1002", frm_str="TST-2")],
            jira_id="1003",
        )
    ],
)
def test_one_entry_that_removes_and_adds_the_same_parent_leaves_it_held(scenario: Scenario) -> None:
    """Jira can list an entry's addition before its removal. The removal sorts
    first, so the parent is still there after the entry."""
    assert _intervals(scenario.links(issue_id="1003")) == [
        ("1003", "parent", "1002", "TST-2", CREATED, 0, DAY_6, None),
        ("1003", "parent", "1002", "TST-2", DAY_6, 1, None, "101"),
    ]


@phased
def test_an_epic_link_change_is_a_parent_change(scenario: Scenario) -> None:
    """Before the epic became the issue's `parent`, it was set through the
    Epic Link field. Jira logged the parent cleared and the Epic Link set to the
    same epic seconds later, so reading only the first would end a parent the
    issue still has."""
    scenario.seed(
        fields=[PARENT_FIELD, EPIC_LINK_FIELD],
        issues=[
            issue("TST-2", jira_id="1002"),
            issue("TST-3", jira_id="1003", fields={PARENT: {"id": "1002", "key": "TST-2"}, EPIC_LINK: None}),
        ],
        events=[
            event("TST-3", 101, "2026-01-06T10:00:00", [item(PARENT, frm="1002", frm_str="TST-2")], jira_id="1003"),
            event(
                "TST-3",
                102,
                "2026-01-06T10:00:14",
                [{**item(EPIC_LINK, to="1002", to_str="TST-2"), "fieldtype": "custom"}],
                jira_id="1003",
            ),
        ],
    )
    yield Build(LINKS_SELECTOR)

    rows = scenario.links(issue_id="1003")
    assert _intervals(rows) == [
        ("1003", "parent", "1002", "TST-2", CREATED, 0, DAY_6, None),
        ("1003", "parent", "1002", "TST-2", "2026-01-06 10:00:14.000", 1, None, "102"),
    ]
    assert {r["evidence"] for r in rows} == {"event"}


@phased
def test_a_link_the_newest_read_no_longer_holds_closes_at_the_issues_last_update(scenario: Scenario) -> None:
    """Nothing in the changelog removed the link; the issue's newest read lacks
    it. The interval closes at the issue's `updated`, the newest change Jira
    recorded on it, and says it was observed."""
    scenario.seed(
        fields=[LINKS_FIELD],
        issues=[issue("TST-1", jira_id="1001", updated=UPDATED), issue("TST-2", jira_id="1002")],
        events=[event("TST-1", 101, "2026-01-06T10:00:00", [_added("TST-2", "blocks")], jira_id="1001")],
    )
    yield Build(LINKS_SELECTOR)

    rows = scenario.links(issue_id="1001")
    assert _intervals(rows) == [("1001", "blocking", "1002", "TST-2", DAY_6, 1, UPDATED, "101")]
    assert (rows[0]["removed_by"], rows[0]["evidence"]) == (None, "observation")


@phased
def test_a_link_gone_before_the_issues_last_update_closes_at_the_read(scenario: Scenario) -> None:
    """The issue was not touched after the link was added — its linked issue
    was deleted, say, which Jira logs on neither side. Only the read bounds the
    removal."""
    scenario.seed(
        fields=[LINKS_FIELD],
        issues=[issue("TST-1", jira_id="1001")],
        events=[event("TST-1", 101, "2026-01-06T10:00:00", [_added("TST-2", "blocks")], jira_id="1001")],
    )
    yield Build(LINKS_SELECTOR)

    assert _intervals(scenario.links(issue_id="1001")) == [("1001", "blocking", None, "TST-2", DAY_6, 1, READ, "101")]


@phased
def test_a_link_the_newest_read_holds_again_reopens_at_its_last_removal(scenario: Scenario) -> None:
    """The changelog removed the link and never added it back, yet the issue
    holds it. It reopens at the removal, a bound rather than a fact."""
    scenario.seed(
        fields=[LINKS_FIELD],
        issues=[
            issue("TST-1", jira_id="1001", fields={LINKS: [_outward("1", BLOCKS, "1002", "TST-2")]}),
            issue("TST-2", jira_id="1002"),
        ],
        events=[
            event("TST-1", 101, "2026-01-06T10:00:00", [_added("TST-2", "blocks")], jira_id="1001"),
            event("TST-1", 102, "2026-01-07T10:00:00", [_removed("TST-2", "blocks")], jira_id="1001"),
        ],
    )
    yield Build(LINKS_SELECTOR)

    rows = scenario.links(issue_id="1001")
    assert _intervals(rows) == [
        ("1001", "blocking", "1002", "TST-2", DAY_6, 1, DAY_7, "101"),
        ("1001", "blocking", "1002", "TST-2", DAY_7, 0, None, None),
    ]
    assert [r["evidence"] for r in rows] == ["event", "observation"]


@phased
def test_a_read_older_than_the_last_event_says_nothing(scenario: Scenario) -> None:
    """The changelog was read after the issue row: the read predates the
    addition, so its missing link is no evidence."""
    scenario.seed(
        fields=[LINKS_FIELD],
        issues=[issue("TST-1", jira_id="1001", extracted_at="2026-01-06T09:00:00"), issue("TST-2", jira_id="1002")],
        events=[event("TST-1", 101, "2026-01-06T10:00:00", [_added("TST-2", "blocks")], jira_id="1001")],
    )
    yield Build(LINKS_SELECTOR)

    assert _intervals(scenario.links(issue_id="1001")) == [("1001", "blocking", "1002", "TST-2", DAY_6, 1, None, "101")]


FIRST_CENSUS = "2026-03-01T12:00:00"
HIDDEN_READ = "2026-04-01T12:00:00"
MISSING_CENSUS = "2026-04-01T13:00:00"
RETURNED_CENSUS = "2026-04-01T14:00:00"
RETURNED_READ = "2026-04-01T15:00:00"
HIDDEN_READ_SQL = "2026-04-01 12:00:00.000"


def _availability(scenario: Scenario) -> str:
    rows = scenario.warehouse.rows(
        "SELECT availability FROM staging.jira__issue_availability_state FINAL"
        " WHERE source_id = {src:String} AND jira_id = '1002'",
        {"src": scenario.source},
    )
    return rows[0]["availability"]


@pytest.mark.parametrize("availability", ["access_lost", "unobserved", "archived", "trashed", "deleted"])
@phased
def test_a_missing_link_only_closes_when_other_issue_availability_allows_it(
    scenario: Scenario, availability: str
) -> None:
    read_at = "2026-03-01T18:00:00" if availability == "access_lost" else HIDDEN_READ
    census_at = read_at if availability == "access_lost" else MISSING_CENSUS
    project_time = FIRST_CENSUS if availability == "access_lost" else MISSING_CENSUS
    project_status = {"archived": "archived", "trashed": "deleted"}.get(availability, "live")
    peers = [
        census_entry("GONE-3", jira_id="1003", extracted_at=MISSING_CENSUS),
        census_entry("GONE-4", jira_id="1004", extracted_at=MISSING_CENSUS),
    ]
    scenario.seed(
        fields=[LINKS_FIELD],
        issues=[
            issue("SEEN-1", jira_id="1001", extracted_at=read_at),
            issue("GONE-2", jira_id="1002", extracted_at=FIRST_CENSUS),
        ],
        events=[event("SEEN-1", 101, "2026-01-06T10:00:00", [_added("GONE-2", "blocks")], jira_id="1001")],
        census=[
            census_entry("SEEN-1", jira_id="1001", extracted_at=census_at),
            census_entry("GONE-2", jira_id="1002", extracted_at=FIRST_CENSUS),
            *(peers if availability == "deleted" else []),
        ],
        projects=[
            project_visibility("SEEN", "1", extracted_at=census_at),
            project_visibility("GONE", "2", status=project_status, extracted_at=project_time),
        ],
    )
    yield Build(LINKS_SELECTOR)

    if availability == "access_lost":
        assert _availability(scenario) == "present"
        scenario.seed(
            fields=[],
            issues=[],
            census=[census_entry("SEEN-1", jira_id="1001", extracted_at=MISSING_CENSUS)],
            projects=[project_visibility("SEEN", "1", extracted_at=MISSING_CENSUS)],
        )
        yield Build(LINKS_SELECTOR, full_refresh=False)

    assert _availability(scenario) == availability
    closes = availability == "deleted"
    rows = scenario.links(issue_id="1001")
    assert _intervals(rows) == [
        ("1001", "blocking", "1002", "GONE-2", DAY_6, 1, HIDDEN_READ_SQL if closes else None, "101")
    ], availability
    assert rows[0]["evidence"] == ("observation" if closes else "event"), availability
    snapshots = scenario.warehouse.rows(
        "SELECT toDateTime64(_tracked_at, 3) > parseDateTime64BestEffort({read:String}, 3) AS tracked_after_read"
        " FROM staging.jira__project_visibility_snapshot WHERE source_id = {src:String} AND project_key = 'GONE'",
        {"src": scenario.source, "read": read_at},
    )
    assert snapshots == [{"tracked_after_read": 1}] * (2 if availability == "access_lost" else 1)


@pytest.mark.parametrize("target_id", [None, "1002"])
@pytest.mark.parametrize("project", ["GONE", "SEEN", "NEVER"])
@phased
def test_a_target_without_availability_uses_project_visibility(
    scenario: Scenario, project: str, target_id: str | None
) -> None:
    target_key = f"{project}-2"
    linked_fields = {LINKS: [_outward("1", BLOCKS, target_id, target_key)]} if target_id is not None else {}
    scenario.seed(
        fields=[LINKS_FIELD],
        issues=[
            issue("SEEN-1", jira_id="1001", extracted_at=FIRST_CENSUS, fields=linked_fields),
            issue("SEEN-3", jira_id="1003", extracted_at=FIRST_CENSUS, fields=linked_fields),
        ],
        events=[event("SEEN-1", 101, "2026-01-06T10:00:00", [_added(target_key, "blocks")], jira_id="1001")],
        projects=[
            project_visibility("SEEN", "1", extracted_at=MISSING_CENSUS),
            project_visibility("GONE", "2", extracted_at=FIRST_CENSUS),
        ],
    )
    yield Build(LINKS_SELECTOR)
    scenario.seed(fields=[], issues=[issue("SEEN-1", jira_id="1001", extracted_at=HIDDEN_READ)])
    yield Build(LINKS_SELECTOR, full_refresh=False)

    rows = scenario.links(issue_id="1001")
    assert _intervals(rows) == [
        ("1001", "blocking", target_id, target_key, DAY_6, 1, None if project == "GONE" else HIDDEN_READ_SQL, "101")
    ], (project, target_id)
    assert rows[0]["evidence"] == ("event" if project == "GONE" else "observation")


@pytest.mark.parametrize("kind", ["issuelink", "parent"])
@pytest.mark.parametrize("with_event", [False, True])
@pytest.mark.parametrize("hidden", [False, True])
@phased
def test_a_read_retains_only_links_whose_other_issue_is_hidden(
    scenario: Scenario, kind: str, with_event: bool, hidden: bool
) -> None:
    fields = {LINKS: [_outward("1", BLOCKS, "1002", "GONE-2")]}
    addition = _added("GONE-2", "blocks")
    catalogue = LINKS_FIELD
    if kind == "parent":
        fields = {PARENT: {"id": "1002", "key": "GONE-2"}}
        addition = item(PARENT, to="1002", to_str="GONE-2")
        catalogue = PARENT_FIELD
    scenario.seed(
        fields=[catalogue],
        issues=[issue("SEEN-1", jira_id="1001", fields=fields), issue("GONE-2", jira_id="1002")],
        events=[event("SEEN-1", 101, "2026-01-06T10:00:00", [addition], jira_id="1001")] if with_event else [],
        census=[census_entry("SEEN-1", jira_id="1001"), census_entry("GONE-2", jira_id="1002")],
        projects=[project_visibility("SEEN", "1"), project_visibility("GONE", "2")],
    )
    yield Build(LINKS_SELECTOR)
    original = scenario.links()
    assert len(original) == (2 if kind == "parent" else 1)

    scenario.seed(
        fields=[],
        issues=[issue("SEEN-1", jira_id="1001", extracted_at=HIDDEN_READ)],
        census=[
            census_entry("SEEN-1", jira_id="1001", extracted_at=MISSING_CENSUS),
            *([] if hidden else [census_entry("GONE-2", jira_id="1002", extracted_at=MISSING_CENSUS)]),
        ],
        projects=[
            project_visibility("SEEN", "1", extracted_at=MISSING_CENSUS),
            *([] if hidden else [project_visibility("GONE", "2", extracted_at=MISSING_CENSUS)]),
        ],
    )
    yield Build(LINKS_SELECTOR, full_refresh=False)

    rows = scenario.links()
    if hidden:
        assert rows == original, (kind, with_event)
    elif with_event:
        assert len(rows) == len(original)
        assert {(r["valid_to"], r["evidence"]) for r in rows} == {(HIDDEN_READ_SQL, "observation")}
    else:
        assert rows == []
    held = scenario.warehouse.rows(
        "SELECT toString(extracted_at) AS extracted_at FROM staging.jira__issue_link_snapshot FINAL"
        " WHERE insight_source_id = {src:String}",
        {"src": scenario.source},
    )
    assert held == ([{"extracted_at": READ}] if hidden else [])

    if not hidden:
        return

    scenario.seed(
        fields=[],
        issues=[],
        census=[
            census_entry("SEEN-1", jira_id="1001", extracted_at=RETURNED_CENSUS),
            census_entry("GONE-2", jira_id="1002", extracted_at=RETURNED_CENSUS),
        ],
        projects=[
            project_visibility("SEEN", "1", extracted_at=RETURNED_CENSUS),
            project_visibility("GONE", "2", extracted_at=RETURNED_CENSUS),
        ],
    )
    yield Build(LINKS_SELECTOR, full_refresh=False)
    assert scenario.links() == original

    returned_held = scenario.warehouse.rows(
        "SELECT toString(extracted_at) AS extracted_at FROM staging.jira__issue_link_snapshot FINAL"
        " WHERE insight_source_id = {src:String}",
        {"src": scenario.source},
    )
    assert returned_held == [{"extracted_at": READ}]

    yield Build(LINKS_SELECTOR, full_refresh=False)
    assert scenario.links() == original
    unchanged_held = scenario.warehouse.rows(
        "SELECT toString(extracted_at) AS extracted_at FROM staging.jira__issue_link_snapshot FINAL"
        " WHERE insight_source_id = {src:String}",
        {"src": scenario.source},
    )
    assert unchanged_held == returned_held

    scenario.seed(fields=[], issues=[issue("SEEN-1", jira_id="1001", fields=fields, extracted_at=RETURNED_READ)])
    yield Build(LINKS_SELECTOR)
    assert scenario.links() == original
    reread_held = scenario.warehouse.rows(
        "SELECT toString(extracted_at) AS extracted_at FROM staging.jira__issue_link_snapshot FINAL"
        " WHERE insight_source_id = {src:String}",
        {"src": scenario.source},
    )
    assert reread_held == [{"extracted_at": "2026-04-01 15:00:00.000"}]


@pytest.mark.parametrize("kind", ["issuelink", "parent"])
@phased
def test_hidden_issue_events_still_open_and_close_links(scenario: Scenario, kind: str) -> None:
    addition = _added("GONE-2", "blocks")
    removal = _removed("GONE-2", "blocks")
    catalogue = LINKS_FIELD
    fields = {LINKS: [_outward("1", BLOCKS, "1002", "GONE-2")]}
    if kind == "parent":
        addition = item(PARENT, to="1002", to_str="GONE-2")
        removal = item(PARENT, frm="1002", frm_str="GONE-2")
        catalogue = PARENT_FIELD
        fields = {PARENT: {"id": "1002", "key": "GONE-2"}}
    scenario.seed(
        fields=[catalogue],
        issues=[issue("SEEN-1", jira_id="1001", fields=fields), issue("GONE-2", jira_id="1002")],
        census=[census_entry("SEEN-1", jira_id="1001"), census_entry("GONE-2", jira_id="1002")],
        projects=[project_visibility("SEEN", "1"), project_visibility("GONE", "2")],
    )
    yield Build(LINKS_SELECTOR)

    scenario.seed(
        fields=[],
        issues=[issue("SEEN-1", jira_id="1001", extracted_at=HIDDEN_READ)],
        events=[event("SEEN-1", 101, "2026-03-30T10:00:00", [addition], jira_id="1001", extracted_at=HIDDEN_READ)],
        census=[census_entry("SEEN-1", jira_id="1001", extracted_at=MISSING_CENSUS)],
        projects=[project_visibility("SEEN", "1", extracted_at=MISSING_CENSUS)],
    )
    yield Build(LINKS_SELECTOR, full_refresh=False)
    rows = scenario.links()
    assert len(rows) == (2 if kind == "parent" else 1)
    assert {(r["valid_from"], r["valid_to"], r["evidence"]) for r in rows} == {
        ("2026-03-30 10:00:00.000", None, "event")
    }

    scenario.seed(
        fields=[],
        issues=[],
        events=[event("SEEN-1", 102, "2026-03-31T10:00:00", [removal], jira_id="1001", extracted_at=HIDDEN_READ)],
    )
    yield Build(LINKS_SELECTOR, full_refresh=False)
    rows = scenario.links()
    assert len(rows) == (2 if kind == "parent" else 1)
    assert {(r["valid_from"], r["valid_to"], r["evidence"], r["origin_event_id"]) for r in rows} == {
        ("2026-03-30 10:00:00.000", "2026-03-31 10:00:00.000", "event", "101")
    }


@pytest.mark.parametrize("kind", ["issuelink", "parent"])
def test_an_issue_without_a_current_read_keeps_no_retained_links(scenario: Scenario, kind: str) -> None:
    catalogue = LINKS_FIELD if kind == "issuelink" else PARENT_FIELD
    fields = (
        {LINKS: [_outward("1", BLOCKS, "1002", "GONE-2")]}
        if kind == "issuelink"
        else {PARENT: {"id": "1002", "key": "GONE-2"}}
    )
    scenario.seed(
        fields=[catalogue],
        issues=[issue("SEEN-1", jira_id="1001", fields=fields), issue("GONE-2", jira_id="1002")],
        census=[
            census_entry("SEEN-1", jira_id="1001", extracted_at=MISSING_CENSUS),
            census_entry("GONE-2", jira_id="1002", extracted_at=FIRST_CENSUS),
        ],
        projects=[
            project_visibility("SEEN", "1", extracted_at=MISSING_CENSUS),
            project_visibility("GONE", "2", extracted_at=FIRST_CENSUS),
        ],
    )
    scenario.build(LINKS_SELECTOR)

    assert _availability(scenario) == "access_lost"
    assert len(scenario.links()) == (2 if kind == "parent" else 1)

    scenario.warehouse.execute("TRUNCATE TABLE bronze_jira.jira_issue")
    scenario.build(LINKS_SELECTOR, full_refresh=False)

    assert (
        scenario.warehouse.rows(
            "SELECT issue_id FROM staging.jira__issue_link_snapshot FINAL WHERE insight_source_id = {src:String}",
            {"src": scenario.source},
        )
        == []
    )
    assert scenario.links() == []


@phased
def test_a_fresh_parent_supersedes_an_earlier_parent_that_is_hidden(scenario: Scenario) -> None:
    scenario.seed(
        fields=[PARENT_FIELD],
        issues=[
            issue("SEEN-1", jira_id="1001", fields={PARENT: {"id": "1002", "key": "GONE-2"}}),
            issue("GONE-2", jira_id="1002"),
            issue("SEEN-3", jira_id="1003"),
        ],
        events=[
            event("SEEN-1", 101, "2026-01-06T10:00:00", [item(PARENT, to="1002", to_str="GONE-2")], jira_id="1001")
        ],
        census=[
            census_entry("SEEN-1", jira_id="1001"),
            census_entry("GONE-2", jira_id="1002"),
            census_entry("SEEN-3", jira_id="1003"),
        ],
        projects=[project_visibility("SEEN", "1"), project_visibility("GONE", "2")],
    )
    yield Build(LINKS_SELECTOR)

    assert _intervals(scenario.links(issue_id="1001")) == [("1001", "parent", "1002", "GONE-2", DAY_6, 1, None, "101")]

    scenario.seed(
        fields=[],
        issues=[
            issue("SEEN-1", jira_id="1001", extracted_at=HIDDEN_READ, fields={PARENT: {"id": "1003", "key": "SEEN-3"}})
        ],
        events=[
            event(
                "SEEN-1",
                102,
                "2026-03-31T10:00:00",
                [item(PARENT, frm="1002", frm_str="GONE-2", to="1003", to_str="SEEN-3")],
                jira_id="1001",
                extracted_at=HIDDEN_READ,
            )
        ],
        census=[
            census_entry("SEEN-1", jira_id="1001", extracted_at=MISSING_CENSUS),
            census_entry("SEEN-3", jira_id="1003", extracted_at=MISSING_CENSUS),
        ],
        projects=[project_visibility("SEEN", "1", extracted_at=MISSING_CENSUS)],
    )
    yield Build(LINKS_SELECTOR, full_refresh=False)

    assert _availability(scenario) == "access_lost"
    held = scenario.warehouse.rows(
        "SELECT kind, target_id, target_key FROM staging.jira__issue_link_snapshot FINAL"
        " WHERE insight_source_id = {src:String} AND issue_id = '1001'",
        {"src": scenario.source},
    )
    assert held == [{"kind": "parent", "target_id": "1003", "target_key": "SEEN-3"}]

    changed_at = "2026-03-31 10:00:00.000"
    rows = scenario.links()
    assert _intervals(rows) == [
        ("1001", "parent", "1002", "GONE-2", DAY_6, 1, changed_at, "101"),
        ("1001", "parent", "1003", "SEEN-3", changed_at, 1, None, "102"),
        ("1002", "sub_issue", "1001", "SEEN-1", DAY_6, 1, changed_at, "101"),
        ("1003", "sub_issue", "1001", "SEEN-1", changed_at, 1, None, "102"),
    ]
    assert {row["evidence"] for row in rows} == {"event"}


@pytest.mark.parametrize("kind", ["issuelink", "parent"])
@pytest.mark.parametrize(
    ("updated", "closed_at"),
    [
        pytest.param("2026-04-01T14:30:00", "2026-04-01 14:30:00.000", id="updated_after_event"),
        pytest.param("2026-04-01T16:00:00", "2026-04-01 15:00:00.000", id="updated_after_read"),
        pytest.param(DAY_6, "2026-04-01 15:00:00.000", id="updated_at_event"),
        pytest.param(None, None, id="read_only"),
    ],
)
@phased
def test_a_missing_link_is_reconciled_after_visibility_returns(
    scenario: Scenario, kind: str, updated: str | None, closed_at: str | None
) -> None:
    with_event = updated is not None
    fields = {LINKS: [_outward("1", BLOCKS, "1002", "GONE-2")]}
    addition = _added("GONE-2", "blocks")
    catalogue = LINKS_FIELD
    if kind == "parent":
        fields = {PARENT: {"id": "1002", "key": "GONE-2"}}
        addition = item(PARENT, to="1002", to_str="GONE-2")
        catalogue = PARENT_FIELD
    scenario.seed(
        fields=[catalogue],
        issues=[issue("SEEN-1", jira_id="1001", fields=fields), issue("GONE-2", jira_id="1002")],
        events=[event("SEEN-1", 101, "2026-01-06T10:00:00", [addition], jira_id="1001")] if with_event else [],
        census=[census_entry("SEEN-1", jira_id="1001"), census_entry("GONE-2", jira_id="1002")],
        projects=[project_visibility("SEEN", "1"), project_visibility("GONE", "2")],
    )
    yield Build(LINKS_SELECTOR)
    original = scenario.links()
    assert len(original) == (2 if kind == "parent" else 1)

    scenario.seed(
        fields=[],
        issues=[issue("SEEN-1", jira_id="1001", extracted_at=HIDDEN_READ)],
        census=[census_entry("SEEN-1", jira_id="1001", extracted_at=MISSING_CENSUS)],
        projects=[project_visibility("SEEN", "1", extracted_at=MISSING_CENSUS)],
    )
    yield Build(LINKS_SELECTOR, full_refresh=False)
    assert scenario.links() == original

    scenario.seed(
        fields=[],
        issues=[],
        census=[
            census_entry("SEEN-1", jira_id="1001", extracted_at=RETURNED_CENSUS),
            census_entry("GONE-2", jira_id="1002", extracted_at=RETURNED_CENSUS),
        ],
        projects=[
            project_visibility("SEEN", "1", extracted_at=RETURNED_CENSUS),
            project_visibility("GONE", "2", extracted_at=RETURNED_CENSUS),
        ],
    )
    yield Build(LINKS_SELECTOR, full_refresh=False)
    assert scenario.links() == original

    scenario.seed(fields=[], issues=[issue("SEEN-1", jira_id="1001", extracted_at=RETURNED_READ, updated=updated)])
    yield Build(LINKS_SELECTOR, full_refresh=False)

    link_type = "parent" if kind == "parent" else "blocking"
    expected = [("1001", link_type, "1002", "GONE-2", DAY_6, 1, closed_at, "101")] if with_event else []
    if with_event and kind == "parent":
        expected.append(("1002", "sub_issue", "1001", "SEEN-1", DAY_6, 1, closed_at, "101"))
    rows = scenario.links()
    assert _intervals(rows) == expected
    if with_event:
        assert {(row["evidence"], row["removed_by"]) for row in rows} == {("observation", None)}
    held = scenario.warehouse.rows(
        "SELECT issue_id FROM staging.jira__issue_link_snapshot FINAL"
        " WHERE insight_source_id = {src:String} AND issue_id = '1001'",
        {"src": scenario.source},
    )
    assert held == []


@pytest.mark.parametrize("kind", ["issuelink", "parent"])
@phased
def test_visibility_returns_only_from_the_first_census_sighting_after_the_gap(scenario: Scenario, kind: str) -> None:
    fields = {LINKS: [_outward("1", BLOCKS, "1002", "GONE-2")]}
    addition = _added("GONE-2", "blocks")
    catalogue = LINKS_FIELD
    if kind == "parent":
        fields = {PARENT: {"id": "1002", "key": "GONE-2"}}
        addition = item(PARENT, to="1002", to_str="GONE-2")
        catalogue = PARENT_FIELD
    scenario.seed(
        fields=[catalogue],
        issues=[issue("SEEN-1", jira_id="1001", fields=fields), issue("GONE-2", jira_id="1002")],
        events=[event("SEEN-1", 101, "2026-01-06T10:00:00", [addition], jira_id="1001")],
        census=[census_entry("SEEN-1", jira_id="1001"), census_entry("GONE-2", jira_id="1002")],
        projects=[project_visibility("SEEN", "1"), project_visibility("GONE", "2")],
    )
    yield Build(LINKS_SELECTOR)

    link_type = "parent" if kind == "parent" else "blocking"
    expected_open = [("1001", link_type, "1002", "GONE-2", DAY_6, 1, None, "101")]
    if kind == "parent":
        expected_open.append(("1002", "sub_issue", "1001", "SEEN-1", DAY_6, 1, None, "101"))
    original = scenario.links()
    assert _intervals(original) == expected_open
    assert {row["evidence"] for row in original} == {"event"}

    scenario.seed(
        fields=[],
        issues=[issue("SEEN-1", jira_id="1001", extracted_at=HIDDEN_READ)],
        census=[census_entry("SEEN-1", jira_id="1001", extracted_at=MISSING_CENSUS)],
        projects=[project_visibility("SEEN", "1", extracted_at=MISSING_CENSUS)],
    )
    yield Build(LINKS_SELECTOR, full_refresh=False)
    assert scenario.links() == original

    before_census_read = "2026-04-01T13:30:00"
    scenario.seed(
        fields=[],
        issues=[issue("SEEN-1", jira_id="1001", extracted_at=before_census_read, updated=before_census_read)],
        census=[
            census_entry("SEEN-1", jira_id="1001", extracted_at=RETURNED_CENSUS),
            census_entry("GONE-2", jira_id="1002", extracted_at=RETURNED_CENSUS),
        ],
        projects=[
            project_visibility("SEEN", "1", extracted_at=RETURNED_CENSUS),
            project_visibility("GONE", "2", extracted_at=RETURNED_CENSUS),
        ],
    )
    yield Build(LINKS_SELECTOR, full_refresh=False)
    assert scenario.links() == original
    held = scenario.warehouse.rows(
        "SELECT toString(extracted_at) AS extracted_at FROM staging.jira__issue_link_snapshot FINAL"
        " WHERE insight_source_id = {src:String} AND issue_id = '1001'",
        {"src": scenario.source},
    )
    assert held == [{"extracted_at": READ}]

    later_census = "2026-04-01T16:00:00"
    scenario.seed(
        fields=[],
        issues=[issue("SEEN-1", jira_id="1001", extracted_at=RETURNED_READ, updated=RETURNED_READ)],
        census=[
            census_entry("SEEN-1", jira_id="1001", extracted_at=later_census),
            census_entry("GONE-2", jira_id="1002", extracted_at=later_census),
        ],
        projects=[
            project_visibility("SEEN", "1", extracted_at=later_census),
            project_visibility("GONE", "2", extracted_at=later_census),
        ],
    )
    yield Build(LINKS_SELECTOR, full_refresh=False)

    closed_at = "2026-04-01 15:00:00.000"
    expected_closed = [("1001", link_type, "1002", "GONE-2", DAY_6, 1, closed_at, "101")]
    if kind == "parent":
        expected_closed.append(("1002", "sub_issue", "1001", "SEEN-1", DAY_6, 1, closed_at, "101"))
    rows = scenario.links()
    assert _intervals(rows) == expected_closed
    assert {(row["evidence"], row["removed_by"]) for row in rows} == {("observation", None)}
    held = scenario.warehouse.rows(
        "SELECT issue_id FROM staging.jira__issue_link_snapshot FINAL"
        " WHERE insight_source_id = {src:String} AND issue_id = '1001'",
        {"src": scenario.source},
    )
    assert held == []


@case(
    fields=[LINKS_FIELD],
    issues=[issue("TST-1", jira_id="1001")],
    events=[
        event("TST-1", 101, "2026-01-06T10:00:00", [_added("ARC-7", "relates to", WORK_ITEM_WORDING)], jira_id="1001"),
        event(
            "TST-1", 102, "2026-01-07T10:00:00", [_removed("ARC-7", "relates to", WORK_ITEM_WORDING)], jira_id="1001"
        ),
    ],
    census=[census_entry("ARC-7", jira_id="9007")],
)
def test_a_linked_issue_outside_the_sync_window_resolves_through_the_census(scenario: Scenario) -> None:
    """ARC-7 is neither synced nor held in any snapshot. The census lists every
    issue a visible project holds, so its key still resolves to its id."""
    assert _intervals(scenario.links()) == [("1001", "connected", "9007", "ARC-7", DAY_6, 1, DAY_7, "101")]
    assert scenario.changelog_states(LINKS, issue="TST-1") == [["9007"], []]
    assert scenario.round_trip_holds()


@phased
def test_a_key_the_census_names_later_reaches_the_journal_without_a_full_refresh(scenario: Scenario) -> None:
    """The journal derives only the issues bronze moved. When the census later
    names the id of a key a link still holds, the owning issue has not moved,
    yet its element must become the id on the next ordinary run."""
    scenario.seed(
        fields=[LINKS_FIELD],
        issues=[issue("TST-1", jira_id="1001")],
        events=[
            event("TST-1", 101, "2026-01-06T10:00:00", [_added("ARC-7", "relates to")], jira_id="1001"),
            event("TST-1", 102, "2026-01-07T10:00:00", [_removed("ARC-7", "relates to")], jira_id="1001"),
        ],
    )
    yield Build(LINKS_SELECTOR)
    assert scenario.changelog_states(LINKS, issue="TST-1") == [["ARC-7"], []]

    scenario.warehouse.insert("bronze_jira.jira_issue_census", [census_entry("ARC-7", jira_id="9007")])
    yield Build(LINKS_SELECTOR, full_refresh=False)

    assert scenario.changelog_states(LINKS, issue="TST-1") == [["9007"], []]
