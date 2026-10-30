"""Two tenants that share a source id, issue ids, keys and changelog ids keep
their own rows in the class producers.

The journal and the link intervals key, delete and resolve by tenant first, so
the same ids under two tenants are two histories, and a readable key resolves
to the issue its own tenant's site gave it.
"""

from __future__ import annotations

from collections.abc import Iterator
from typing import Any

import pytest
from conftest import LINKS_SELECTOR, Scenario
from helpers import LATER_SYNC, OTHER_TENANT, SOURCE_ID, TENANT_ID, as_other_tenant, event, field, issue, item

POINTS = "customfield_10001"
POINTS_FIELD = field(POINTS, name="Story Points", schema_type="number")
LINKS = "issuelinks"
LINKS_FIELD = field(LINKS, name="Linked Issues", schema_type="array", schema_items="issuelinks")

BLOCKS = {"id": "10000", "name": "Blocks", "outward": "blocks", "inward": "is blocked by"}
DUPLICATE = {"id": "10001", "name": "Duplicate", "outward": "duplicates", "inward": "is duplicated by"}

MINE_CREATED = "2026-01-05T09:00:00"
THEIRS_CREATED = "2026-01-04T09:00:00"
DAY_6 = "2026-01-06T10:00:00"
DAY_7 = "2026-01-07T10:00:00"
DAY_8 = "2026-01-08T10:00:00"
MINE_OPENED = "2026-01-05 09:00:00.000"
MINE_CLOSED = "2026-01-07 10:00:00.000"
THEIRS_OPENED = "2026-01-06 10:00:00.000"

POINTS_ROWS = (
    " WHERE insight_source_id = {src:String} AND data_source = 'jira' AND field_id = {field:String}"
    " ORDER BY tenant_id, issue_id, event_order, unique_key"
)
JOURNAL_SQL = (
    "SELECT tenant_id, issue_id, event_kind, event_id, value_ids, unique_key, _version"
    " FROM staging.jira__field_history_derived FINAL" + POINTS_ROWS
)
CLASS_SQL = (
    "SELECT tenant_id, issue_id, event_kind, event_id, value_ids, unique_key, _version"
    " FROM silver.class_task_field_history FINAL" + POINTS_ROWS
)


def _points_change(changelog_id: int, at: str, frm: str, to: str, **extra: Any) -> dict[str, Any]:
    return event("TST-1", changelog_id, at, [item(POINTS, frm=frm, frm_str=frm, to=to, to_str=to)], **extra)


def _points(scenario: Scenario, sql: str) -> list[dict[str, Any]]:
    return scenario.warehouse.rows(sql, {"src": SOURCE_ID, "field": POINTS})


def _history(rows: list[dict[str, Any]]) -> list[tuple[Any, ...]]:
    return [(r["tenant_id"], r["event_kind"], r["event_id"], list(r["value_ids"])) for r in rows]


def _clear_class(scenario: Scenario) -> None:
    scenario.warehouse.execute(
        "DELETE FROM silver.class_task_field_history WHERE insight_source_id = {src:String}"
        " SETTINGS mutations_sync = 2",
        {"src": SOURCE_ID},
    )


@pytest.fixture
def _own_class_rows(scenario: Scenario) -> Iterator[None]:
    """The class table outlives the test, and another test's rows of this source raise its boundary."""
    _clear_class(scenario)
    yield
    _clear_class(scenario)


@pytest.mark.usefixtures("_own_class_rows")
def test_the_same_issue_and_changelog_ids_are_one_history_per_tenant_through_the_class(scenario: Scenario) -> None:
    """Issue 1001 and entry 101 exist in both tenants with different values. A
    later sync touches only the other tenant's issue: rebuilding it must leave
    the first tenant's rows as they were."""
    scenario.seed(
        fields=[POINTS_FIELD],
        issues=[issue("TST-1", jira_id="1001", fields={POINTS: 5})],
        events=[_points_change(101, DAY_6, "3", "5", jira_id="1001")],
    )
    scenario.seed(
        fields=as_other_tenant([POINTS_FIELD]),
        issues=as_other_tenant([issue("TST-1", jira_id="1001", fields={POINTS: 9})]),
        events=as_other_tenant([_points_change(101, DAY_6, "7", "9", jira_id="1001")]),
    )
    scenario.build()
    first = _points(scenario, JOURNAL_SQL)

    assert _history(first) == [
        (TENANT_ID, "synthetic_initial", "initial:1001", ["3"]),
        (TENANT_ID, "changelog", "101", ["5"]),
        (OTHER_TENANT, "synthetic_initial", "initial:1001", ["7"]),
        (OTHER_TENANT, "changelog", "101", ["9"]),
    ]
    assert all(r["unique_key"].startswith(f"{r['tenant_id']}-{SOURCE_ID}-jira-1001-") for r in first), first

    scenario.warehouse.insert(
        "bronze_jira.jira_issue",
        as_other_tenant([issue("TST-1", jira_id="1001", fields={POINTS: 11}, extracted_at=LATER_SYNC)]),
    )
    scenario.warehouse.insert(
        "bronze_jira.jira_issue_history",
        as_other_tenant([_points_change(102, DAY_8, "9", "11", jira_id="1001", extracted_at=LATER_SYNC)]),
    )
    scenario.build(full_refresh=False)
    second = _points(scenario, JOURNAL_SQL)

    assert [r for r in second if r["tenant_id"] == TENANT_ID] == [r for r in first if r["tenant_id"] == TENANT_ID], (
        "rebuilding the other tenant's issue must not touch this tenant's rows"
    )
    assert _history([r for r in second if r["tenant_id"] == OTHER_TENANT]) == [
        (OTHER_TENANT, "synthetic_initial", "initial:1001", ["7"]),
        (OTHER_TENANT, "changelog", "101", ["9"]),
        (OTHER_TENANT, "changelog", "102", ["11"]),
    ]

    scenario.warehouse.dbt("run", "--select", "class_task_field_history")

    assert _history(_points(scenario, CLASS_SQL)) == _history(second)


def test_a_batched_rebuild_of_one_tenants_issues_leaves_the_others_alone(scenario: Scenario) -> None:
    """Both tenants hold issues 1001 and 1002. A later sync touches the other
    tenant's two issues, and each is rebuilt in a batch of its own: every batch
    replaces only its own tenant's rows."""
    issues = [issue(f"TST-{n}", jira_id=f"100{n}", fields={POINTS: n}) for n in (1, 2)]
    scenario.seed(fields=[POINTS_FIELD], issues=issues)
    scenario.seed(fields=as_other_tenant([POINTS_FIELD]), issues=as_other_tenant(issues))
    scenario.build()
    mine = [r for r in _points(scenario, JOURNAL_SQL) if r["tenant_id"] == TENANT_ID]

    scenario.warehouse.insert(
        "bronze_jira.jira_issue",
        as_other_tenant(
            [issue(f"TST-{n}", jira_id=f"100{n}", fields={POINTS: n + 10}, extracted_at=LATER_SYNC) for n in (1, 2)]
        ),
    )
    scenario.build(full_refresh=False, dbt_vars={"jira_journal_issues_per_batch": 1})
    after = _points(scenario, JOURNAL_SQL)

    theirs = [r for r in after if r["tenant_id"] == OTHER_TENANT]

    assert mine, "the scenario produced no journal"
    assert [r for r in after if r["tenant_id"] == TENANT_ID] == mine, (
        "another tenant's batches must not touch these rows"
    )
    assert {r["issue_id"] for r in theirs} == {"1001", "1002"}
    assert all(r["_version"] > mine[0]["_version"] for r in theirs), "both of the other tenant's issues were rebuilt"


def _outward(link_id: str, link_type: dict[str, str], issue_id: str, key: str) -> dict[str, Any]:
    return {"id": link_id, "type": link_type, "outwardIssue": {"id": issue_id, "key": key}}


def test_each_tenant_folds_its_links_against_its_own_issues(scenario: Scenario) -> None:
    """Both tenants name issue 1001 TST-1 and log entry 101 on it, but TST-2 is
    1002 in one site and 2002 in the other, and 1001 was created on different
    days. The first tenant's link was held before its removal, so it opens at
    its own issue's creation; the other's opens at its own addition."""
    scenario.seed(
        fields=[LINKS_FIELD],
        issues=[
            issue("TST-1", jira_id="1001", created=MINE_CREATED, fields={LINKS: []}),
            issue("TST-2", jira_id="1002", created=MINE_CREATED),
        ],
        events=[
            event("TST-1", 101, DAY_7, [item(LINKS, frm="TST-2", frm_str="This issue blocks TST-2")], jira_id="1001")
        ],
    )
    scenario.seed(
        fields=as_other_tenant([LINKS_FIELD]),
        issues=as_other_tenant(
            [
                issue(
                    "TST-1",
                    jira_id="1001",
                    created=THEIRS_CREATED,
                    fields={LINKS: [_outward("7", DUPLICATE, "2002", "TST-2")]},
                ),
                issue("TST-2", jira_id="2002", created=THEIRS_CREATED),
            ]
        ),
        events=as_other_tenant(
            [
                event(
                    "TST-1", 101, DAY_6, [item(LINKS, to="TST-2", to_str="This issue duplicates TST-2")], jira_id="1001"
                )
            ]
        ),
    )
    scenario.build(LINKS_SELECTOR)

    rows = scenario.warehouse.rows(
        "SELECT tenant_id, issue_id, id_readable, link_type, target_id, target_readable,"
        "       toString(valid_from) AS valid_from, valid_from_known,"
        "       if(valid_to IS NULL, NULL, toString(valid_to)) AS valid_to, unique_key"
        " FROM staging.jira__task_links FINAL WHERE insight_source_id = {src:String}"
        " ORDER BY tenant_id, issue_id, valid_from",
        {"src": SOURCE_ID},
    )

    assert [tuple(v for k, v in r.items() if k != "unique_key") for r in rows] == [
        (TENANT_ID, "1001", "TST-1", "blocking", "1002", "TST-2", MINE_OPENED, 0, MINE_CLOSED),
        (OTHER_TENANT, "1001", "TST-1", "duplicate_of", "2002", "TST-2", THEIRS_OPENED, 1, None),
    ]
    assert all(r["unique_key"].startswith(f"{r['tenant_id']}-{SOURCE_ID}-jira-1001-") for r in rows), rows

    linked = scenario.warehouse.rows(
        "SELECT tenant_id, arraySort(groupUniqArrayArray(value_ids)) AS linked"
        " FROM staging.jira__field_history_derived FINAL"
        " WHERE insight_source_id = {src:String} AND field_id = {field:String}"
        " GROUP BY tenant_id ORDER BY tenant_id",
        {"src": SOURCE_ID, "field": LINKS},
    )
    assert [(r["tenant_id"], list(r["linked"])) for r in linked] == [(TENANT_ID, ["1002"]), (OTHER_TENANT, ["2002"])], (
        "the journal resolves each tenant's TST-2 to its own issue"
    )
