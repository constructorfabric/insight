"""Two tenants that share a source id, issue ids, keys and changelog ids close
and reopen their links from their own issue reads.

`jira__task_links` lets the issue's newest read close or reopen an occurrence
its events leave in the other state, and reads an Epic Link change as a parent
change by the field catalogue. Both lookups are by tenant first, so one
tenant's read or catalogue never decides the other tenant's link.
"""

from __future__ import annotations

from typing import Any

from conftest import LINKS_SELECTOR, Scenario
from helpers import LATER_SYNC, OTHER_TENANT, SOURCE_ID, TENANT_ID, as_other_tenant, event, field, issue, item

LINKS = "issuelinks"
LINKS_FIELD = field(LINKS, name="Linked Issues", schema_type="array", schema_items="issuelinks")
PARENT = "parent"
PARENT_FIELD = field(PARENT, name="Parent")
EPIC_LINK = "customfield_11300"
EPIC_LINK_FIELD = field(
    EPIC_LINK, name="Epic Link", schema_type="any", schema_custom="com.pyxis.greenhopper.jira:gh-epic-link"
)
UNRELATED_FIELD = field(EPIC_LINK, name="Reviewer Issue", schema_type="string")

BLOCKS = {"id": "10000", "name": "Blocks", "outward": "blocks", "inward": "is blocked by"}

DAY_6 = "2026-01-06T10:00:00"
DAY_7 = "2026-01-07T10:00:00"
MINE_UPDATED = "2026-02-01T09:00:00"
THEIRS_UPDATED = "2026-02-15T09:00:00"

CREATED_AT = "2026-01-05 09:00:00.000"
DAY_6_AT = "2026-01-06 10:00:00.000"
DAY_7_AT = "2026-01-07 10:00:00.000"
MINE_UPDATED_AT = "2026-02-01 09:00:00.000"

LINK_ROWS = (
    "SELECT tenant_id, issue_id, link_type, toString(valid_from) AS valid_from, valid_from_known,"
    "       if(valid_to IS NULL, NULL, toString(valid_to)) AS valid_to, toString(evidence) AS evidence"
    " FROM staging.jira__task_links FINAL"
    " WHERE insight_source_id = {src:String} AND issue_id = {issue:String}"
    " ORDER BY tenant_id, link_type, valid_from"
)


def _outward(link_id: str, issue_id: str, key: str) -> dict[str, Any]:
    return {"id": link_id, "type": BLOCKS, "outwardIssue": {"id": issue_id, "key": key}}


def _added(key: str) -> dict[str, Any]:
    return item(LINKS, to=key, to_str=f"This issue blocks {key}")


def _removed(key: str) -> dict[str, Any]:
    return item(LINKS, frm=key, frm_str=f"This issue blocks {key}")


def _links(scenario: Scenario, issue_id: str) -> list[tuple[Any, ...]]:
    rows = scenario.warehouse.rows(LINK_ROWS, {"src": SOURCE_ID, "issue": issue_id})
    return [tuple(r.values()) for r in rows]


def test_one_tenants_newest_read_does_not_close_the_others_link(scenario: Scenario) -> None:
    """Both tenants log entry 101 adding the same link on issue 1001. Only this
    tenant's read lacks it, so only this tenant's link closes, at its own
    issue's `updated`. The other tenant read later, with a later `updated`; had
    that read been joined to this tenant's link it would win by version and
    move the close."""
    events = [event("TST-1", 101, DAY_6, [_added("TST-2")], jira_id="1001")]
    scenario.seed(
        fields=[LINKS_FIELD],
        issues=[issue("TST-1", jira_id="1001", updated=MINE_UPDATED), issue("TST-2", jira_id="1002")],
        events=events,
    )
    scenario.seed(
        fields=as_other_tenant([LINKS_FIELD]),
        issues=as_other_tenant(
            [
                issue(
                    "TST-1",
                    jira_id="1001",
                    updated=THEIRS_UPDATED,
                    extracted_at=LATER_SYNC,
                    fields={LINKS: [_outward("1", "1002", "TST-2")]},
                ),
                issue("TST-2", jira_id="1002"),
            ]
        ),
        events=as_other_tenant(events),
    )
    scenario.build(LINKS_SELECTOR)

    assert _links(scenario, "1001") == [
        (TENANT_ID, "1001", "blocking", DAY_6_AT, 1, MINE_UPDATED_AT, "observation"),
        (OTHER_TENANT, "1001", "blocking", DAY_6_AT, 1, None, "event"),
    ]


def test_one_tenants_newest_read_does_not_reopen_the_others_link(scenario: Scenario) -> None:
    """Both tenants add and then remove the same link. This tenant's read holds
    it again and reopens it at the removal; the other tenant's read does not,
    and its link stays closed by its own event."""
    events = [
        event("TST-1", 101, DAY_6, [_added("TST-2")], jira_id="1001"),
        event("TST-1", 102, DAY_7, [_removed("TST-2")], jira_id="1001"),
    ]
    scenario.seed(
        fields=[LINKS_FIELD],
        issues=[
            issue("TST-1", jira_id="1001", fields={LINKS: [_outward("1", "1002", "TST-2")]}),
            issue("TST-2", jira_id="1002"),
        ],
        events=events,
    )
    scenario.seed(
        fields=as_other_tenant([LINKS_FIELD]),
        issues=as_other_tenant(
            [issue("TST-1", jira_id="1001", extracted_at=LATER_SYNC), issue("TST-2", jira_id="1002")]
        ),
        events=as_other_tenant(events),
    )
    scenario.build(LINKS_SELECTOR)

    assert _links(scenario, "1001") == [
        (TENANT_ID, "1001", "blocking", DAY_6_AT, 1, DAY_7_AT, "event"),
        (TENANT_ID, "1001", "blocking", DAY_7_AT, 0, None, "observation"),
        (OTHER_TENANT, "1001", "blocking", DAY_6_AT, 1, DAY_7_AT, "event"),
    ]


def test_an_epic_link_field_is_read_from_its_own_tenants_catalogue(scenario: Scenario) -> None:
    """Field customfield_11300 is the Epic Link in this tenant's site and an
    unrelated field in the other's. Both issues hold epic TST-2 as their parent
    and log entry 101 setting the field to it; only this tenant's entry is a
    parent change, so the other tenant's parent is held from before any event."""
    issues = [
        issue("TST-2", jira_id="1002"),
        issue("TST-3", jira_id="1003", fields={PARENT: {"id": "1002", "key": "TST-2"}, EPIC_LINK: None}),
    ]
    events = [
        event(
            "TST-3", 101, DAY_6, [{**item(EPIC_LINK, to="1002", to_str="TST-2"), "fieldtype": "custom"}], jira_id="1003"
        )
    ]
    scenario.seed(fields=[PARENT_FIELD, EPIC_LINK_FIELD], issues=issues, events=events)
    scenario.seed(
        fields=as_other_tenant([PARENT_FIELD, UNRELATED_FIELD]),
        issues=as_other_tenant(issues),
        events=as_other_tenant(events),
    )
    scenario.build(LINKS_SELECTOR)

    assert _links(scenario, "1003") == [
        (TENANT_ID, "1003", "parent", DAY_6_AT, 1, None, "event"),
        (OTHER_TENANT, "1003", "parent", CREATED_AT, 0, None, "event"),
    ]
