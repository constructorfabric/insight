"""Two tenants that share a source id, issue ids and changelog ids stay apart.

A source id names a connector instance inside one tenant only, and Jira ids are
unique only inside one site, so two tenants can hold the very same ids with
different contents. Every model up to the class boundary keys, joins, groups and
ranks by tenant first; these seed the collision and read each model per tenant.
"""

from __future__ import annotations

import json
from typing import Any

from conftest import LINKS_SELECTOR, Scenario
from helpers import OTHER_TENANT, SOURCE_ID, TENANT_ID, as_other_tenant, event, field, issue, item

POINTS = "customfield_10001"
LABEL = "customfield_10002"
CHOICE = "customfield_10005"
NOTES = "customfield_10010"
STATUS = "status"
LINKS = "issuelinks"

BLOCKS = {"id": "10000", "name": "Blocks", "outward": "blocks", "inward": "is blocked by"}
GATES = {"id": "10500", "name": "Example Gate", "outward": "blocks", "inward": "is gated by"}

SAME_INSTANT = "2026-01-06T10:00:00"
STATUS_NAMES = {"1": "Open", "3": "In Progress", "6": "Closed"}
TEXTAREA = "com.atlassian.jira.plugin.system.customfieldtypes:textarea"
NOTES_BODY = {
    "type": "doc",
    "version": 1,
    "content": [{"type": "paragraph", "content": [{"type": "text", "text": "mine now"}]}],
}


def _seed(scenario: Scenario) -> None:
    mine = {
        "fields": [
            field(POINTS, name="Story Points", schema_type="number"),
            field(LABEL, name="Label", schema_type="string"),
        ],
        "issues": [issue("TST-1", fields={POINTS: 5, LABEL: "mine"})],
        "events": [
            event("TST-1", 101, "2026-01-06T10:00:00", [item(POINTS, frm="3", frm_str="3", to="5", to_str="5")])
        ],
    }
    theirs = {
        "fields": [field(POINTS, name="Story Points", schema_type="number")],
        "issues": [issue("TST-1", fields={POINTS: 9})],
        "events": [
            event(
                "TST-1",
                101,
                "2026-01-06T10:00:00",
                [item(POINTS, frm="7", frm_str="7", to="9", to_str="9"), item(LABEL, to="x", to_str="theirs")],
            )
        ],
    }
    scenario.seed(**mine)
    scenario.seed(**{name: as_other_tenant(rows) for name, rows in theirs.items()})
    scenario.build()


def _rows(scenario: Scenario, sql: str, **parameters: str) -> list[dict[str, Any]]:
    return scenario.warehouse.rows(sql, {"src": SOURCE_ID, **parameters})


def test_each_tenant_keeps_its_own_changelog_items(scenario: Scenario) -> None:
    _seed(scenario)

    rows = _rows(
        scenario,
        "SELECT tenant_id, field_id, value_to_string FROM staging.jira_changelog_items"
        " WHERE insight_source_id = {src:String} ORDER BY tenant_id, field_id",
    )

    assert [(r["tenant_id"], r["field_id"], r["value_to_string"]) for r in rows] == [
        (TENANT_ID, POINTS, "5"),
        (OTHER_TENANT, POINTS, "9"),
        (OTHER_TENANT, LABEL, "theirs"),
    ]


def test_a_field_is_classified_only_where_its_tenant_catalogues_it(scenario: Scenario) -> None:
    _seed(scenario)

    kinds = _rows(
        scenario,
        "SELECT tenant_id, field_id FROM staging.jira__task_field_kind"
        " WHERE insight_source_id = {src:String} ORDER BY tenant_id, field_id",
    )
    unclassified = _rows(
        scenario,
        "SELECT tenant_id, field_id FROM staging.jira__task_field_unclassified WHERE insight_source_id = {src:String}",
    )

    assert [(r["tenant_id"], r["field_id"]) for r in kinds] == [
        (TENANT_ID, POINTS),
        (TENANT_ID, LABEL),
        (OTHER_TENANT, POINTS),
    ]
    assert [(r["tenant_id"], r["field_id"]) for r in unclassified] == [(OTHER_TENANT, LABEL)], (
        "the label is catalogued by the first tenant only; the other's change to it is unclassified there"
    )


def test_each_tenant_snapshots_its_own_issue(scenario: Scenario) -> None:
    _seed(scenario)

    rows = _rows(
        scenario,
        "SELECT tenant_id, value_ids FROM staging.jira_issue_field_snapshot"
        " WHERE insight_source_id = {src:String} AND field_id = {field:String} ORDER BY tenant_id",
        field=POINTS,
    )

    assert [(r["tenant_id"], list(r["value_ids"])) for r in rows] == [(TENANT_ID, ["5"]), (OTHER_TENANT, ["9"])]


def test_each_tenant_resolves_an_issue_key_to_its_own_issue(scenario: Scenario) -> None:
    """The same readable key names a different issue in each tenant's site."""
    catalogue = [field(POINTS, name="Story Points", schema_type="number")]
    scenario.seed(fields=catalogue, issues=[issue("TST-1", fields={POINTS: 5}, jira_id="1001")])
    scenario.seed(
        fields=as_other_tenant(catalogue), issues=as_other_tenant([issue("TST-1", fields={POINTS: 9}, jira_id="2002")])
    )
    scenario.build()

    rows = _rows(
        scenario,
        "SELECT tenant_id, issue_id FROM staging.jira__issue_key_ids"
        " WHERE insight_source_id = {src:String} AND issue_key = 'TST-1' ORDER BY tenant_id",
    )

    assert [(r["tenant_id"], r["issue_id"]) for r in rows] == [(TENANT_ID, "1001"), (OTHER_TENANT, "2002")]


def test_each_tenant_converts_a_shared_field_id_by_its_own_kind(scenario: Scenario) -> None:
    """One tenant catalogues the field as a number, the other as a single option."""
    scenario.seed(
        fields=[field(CHOICE, name="Example Choice", schema_type="number")], issues=[issue("TST-1", fields={CHOICE: 5})]
    )
    scenario.seed(
        fields=as_other_tenant([field(CHOICE, name="Example Choice", schema_type="option")]),
        issues=as_other_tenant([issue("TST-1", fields={CHOICE: {"id": "9002", "value": "High"}})]),
    )
    scenario.build()

    kinds = _rows(
        scenario,
        "SELECT tenant_id, field_kind FROM staging.jira__task_field_kind"
        " WHERE insight_source_id = {src:String} AND field_id = {field:String} ORDER BY tenant_id",
        field=CHOICE,
    )
    values = _rows(
        scenario,
        "SELECT tenant_id, value_ids, value_displays FROM staging.jira_issue_field_snapshot"
        " WHERE insight_source_id = {src:String} AND field_id = {field:String} ORDER BY tenant_id, value_ids",
        field=CHOICE,
    )

    assert [(r["tenant_id"], r["field_kind"]) for r in kinds] == [(TENANT_ID, "scalar"), (OTHER_TENANT, "option")]
    assert [(r["tenant_id"], list(r["value_ids"]), list(r["value_displays"])) for r in values] == [
        (TENANT_ID, ["5"], ["5"]),
        (OTHER_TENANT, ["9002"], ["High"]),
    ]


def _status_change(key: str, changelog_id: int, frm: str, to: str) -> dict[str, Any]:
    return event(
        key,
        changelog_id,
        SAME_INSTANT,
        [item(STATUS, frm=frm, frm_str=STATUS_NAMES[frm], to=to, to_str=STATUS_NAMES[to])],
    )


def test_each_tenant_ranks_its_tied_entries_by_its_own_chain(scenario: Scenario) -> None:
    """Both tenants tie entries 101 and 102 of TST-1 but chain them in opposite
    orders. On TST-2 only the first tenant ties two entries; the other's 201 is
    alone at that instant and so has no rank. The other tenant's tie on TST-3
    changes a field only the first tenant catalogues, so it ranks nothing."""
    catalogue = [field(STATUS, name="Status", schema_type="status")]
    closed = {STATUS: {"id": "6", "name": "Closed"}}
    scenario.seed(
        fields=[*catalogue, field(LABEL, name="Label", schema_type="string")],
        issues=[issue("TST-1", fields=closed), issue("TST-2", fields=closed)],
        events=[
            _status_change("TST-1", 101, "3", "6"),
            _status_change("TST-1", 102, "1", "3"),
            _status_change("TST-2", 201, "1", "3"),
            _status_change("TST-2", 202, "3", "6"),
        ],
    )
    scenario.seed(
        fields=as_other_tenant(catalogue),
        issues=as_other_tenant(
            [
                issue("TST-1", fields=closed),
                issue("TST-2", fields={STATUS: {"id": "3", "name": "In Progress"}}),
                issue("TST-3", fields={LABEL: "c"}),
            ]
        ),
        events=as_other_tenant(
            [
                _status_change("TST-1", 101, "1", "3"),
                _status_change("TST-1", 102, "3", "6"),
                _status_change("TST-2", 201, "1", "3"),
                event("TST-3", 301, SAME_INSTANT, [item(LABEL, frm_str="a", to_str="b")]),
                event("TST-3", 302, SAME_INSTANT, [item(LABEL, frm_str="b", to_str="c")]),
            ]
        ),
    )
    scenario.build()

    rows = _rows(
        scenario,
        "SELECT tenant_id, issue_id, changelog_id, entry_rank FROM staging.jira__changelog_entry_ranks"
        " WHERE insight_source_id = {src:String} ORDER BY tenant_id, issue_id, changelog_id",
    )

    assert [(r["tenant_id"], r["issue_id"], r["changelog_id"], r["entry_rank"]) for r in rows] == [
        (TENANT_ID, "TST-1", "101", 1),
        (TENANT_ID, "TST-1", "102", 0),
        (TENANT_ID, "TST-2", "201", 0),
        (TENANT_ID, "TST-2", "202", 1),
        (OTHER_TENANT, "TST-1", "101", 0),
        (OTHER_TENANT, "TST-1", "102", 1),
    ]


def test_long_text_bodies_come_only_from_the_tenant_that_classifies_the_field_as_long_text(scenario: Scenario) -> None:
    """The side table is content-addressed and carries no tenant, so what is
    tenant-visible is which bodies enter it: the other tenant's plain string
    field of the same id contributes none, from its issue or its changelog."""
    scenario.seed(
        fields=[field(NOTES, name="Example Notes", schema_type="string", schema_custom=TEXTAREA)],
        issues=[issue("TST-1", fields={NOTES: NOTES_BODY})],
        events=[event("TST-1", 101, SAME_INSTANT, [item(NOTES, frm_str="mine before", to_str="mine now")])],
    )
    scenario.seed(
        fields=as_other_tenant([field(NOTES, name="Example Notes", schema_type="string")]),
        issues=as_other_tenant([issue("TST-1", fields={NOTES: "theirs now"})]),
        events=as_other_tenant(
            [
                event(
                    "TST-1",
                    101,
                    SAME_INSTANT,
                    [item(NOTES, frm="theirs before", frm_str="theirs before", to="theirs now", to_str="theirs now")],
                )
            ]
        ),
    )
    scenario.build()

    bodies = [
        (r["content_form"], json.loads(r["content"]) if r["content_form"] == "adf_json" else r["content"])
        for r in scenario.text_rows()
    ]

    assert bodies == [("adf_json", NOTES_BODY), ("rendered_text", "mine before"), ("rendered_text", "mine now")]


def _holds(link_id: str, link_type: dict[str, str], issue_id: str, key: str) -> dict[str, Any]:
    return {LINKS: [{"id": link_id, "type": link_type, "outwardIssue": {"id": issue_id, "key": key}}]}


def _adds(key: str, changelog_id: int, at: str, target: str, jira_id: str) -> dict[str, Any]:
    return event(key, changelog_id, at, [item(LINKS, to=target, to_str=f"This issue blocks {target}")], jira_id=jira_id)


def test_each_tenant_resolves_its_link_occurrences_within_the_tenant(scenario: Scenario) -> None:
    """Both tenants share issue ids, keys and changelog ids, and log the same
    phrase. In the first it means type 10000; in the other it means type
    10500, held twice, so a phrase resolved across tenants would take 10500 for
    the first tenant's unheld TST-3 link, and a key resolved across tenants
    would count each operation twice."""
    links = [field(LINKS, name="Linked Issues", schema_type="array", schema_items="issuelinks")]
    scenario.seed(
        fields=links,
        issues=[
            issue("TST-1", jira_id="1001", fields=_holds("1", BLOCKS, "1002", "TST-2")),
            issue("TST-2", jira_id="1002"),
            issue("TST-3", jira_id="1003"),
        ],
        events=[
            _adds("TST-1", 101, "2026-01-06T10:00:00", "TST-2", "1001"),
            _adds("TST-1", 102, "2026-01-07T10:00:00", "TST-3", "1001"),
        ],
    )
    scenario.seed(
        fields=as_other_tenant(links),
        issues=as_other_tenant(
            [
                issue("TST-1", jira_id="1001", fields=_holds("1", GATES, "1002", "TST-2")),
                issue("TST-2", jira_id="1002"),
                issue("TST-4", jira_id="1004", fields=_holds("2", GATES, "1002", "TST-2")),
            ]
        ),
        events=as_other_tenant([_adds("TST-1", 101, "2026-01-06T10:00:00", "TST-2", "1001")]),
    )
    scenario.build(LINKS_SELECTOR)

    rows = _rows(
        scenario,
        "SELECT tenant_id, issue_id, occurrence_key, target_ident, length(ops) AS op_count, held_before"
        " FROM staging.jira__link_occurrences FINAL"
        " WHERE insight_source_id = {src:String} ORDER BY tenant_id, issue_id, target_ident",
    )

    assert [
        (r["tenant_id"], r["issue_id"], r["occurrence_key"], r["target_ident"], r["op_count"], r["held_before"])
        for r in rows
    ] == [
        (TENANT_ID, "1001", "type:10000:outward", "1002", 1, False),
        (TENANT_ID, "1001", "type:10000:outward", "1003", 1, False),
        (OTHER_TENANT, "1001", "type:10500:outward", "1002", 1, False),
        (OTHER_TENANT, "1004", "type:10500:outward", "1002", 0, True),
    ]
