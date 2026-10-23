"""Comment and worklog lifecycle events stay with their own tenant's issue.

Comment and worklog ids are unique only inside one Jira site, so two tenants can
hold the same id on different issues, and the same issue id with different
entities on it. Each event carries the tenant of the issue it lands on.
"""

from __future__ import annotations

import pytest
from conftest import LIFECYCLE_SELECTOR, Scenario
from helpers import OTHER_TENANT, SOURCE_ID, TENANT_ID, as_other_tenant, comment, issue, worklog

SAME_INSTANT = "2026-01-06T10:00:00.000+0000"

# The rank among one issue's lifecycle events of one millisecond is the low
# digits of `event_order` (`task_event_order`).
COMMENT_EVENTS_SQL = (
    "SELECT tenant_id, issue_id, id_readable, value_ids[1] AS entity_id, author_id, delta_action,"
    " toUInt32(event_order % 100000) AS same_instant_rank"
    " FROM staging.jira__comment_lifecycle_events FINAL WHERE insight_source_id = {src:String}"
)
WORKLOG_EVENTS_SQL = (
    "SELECT tenant_id, issue_id, id_readable, value_ids[1] AS entity_id, author_id, delta_action,"
    " toUInt32(event_order % 100000) AS same_instant_rank"
    " FROM staging.jira__worklog_lifecycle_events FINAL WHERE insight_source_id = {src:String}"
)

ENTITIES = {"comment": (comment, "comments", COMMENT_EVENTS_SQL), "worklog": (worklog, "worklogs", WORKLOG_EVENTS_SQL)}


@pytest.mark.parametrize("entity", sorted(ENTITIES))
def test_each_event_lands_on_its_own_tenants_issue_ranked_among_its_own_tenants_events(
    scenario: Scenario, entity: str
) -> None:
    """Entity 501 sits on issue 1001 in one tenant and on 2002 in the other.
    Issue 2002 holds 503 in the first tenant and 501 and 502 in the second, all
    at one millisecond: ranked together, 503 would come third."""
    build, stream, events_sql = ENTITIES[entity]
    scenario.seed(
        fields=[],
        issues=[issue("TST-1", jira_id="1001"), issue("TST-2", jira_id="2002")],
        **{
            stream: [
                build("TST-1", 501, jira_id="1001", updated=SAME_INSTANT, author="author-mine"),
                build("TST-2", 503, jira_id="2002", updated=SAME_INSTANT, author="author-mine"),
            ]
        },
    )
    scenario.seed(
        fields=[],
        issues=as_other_tenant([issue("OTH-7", jira_id="2002")]),
        **{
            stream: as_other_tenant(
                [
                    build("OTH-7", 501, jira_id="2002", updated=SAME_INSTANT, author="author-theirs"),
                    build("OTH-7", 502, jira_id="2002", updated=SAME_INSTANT, author="author-theirs"),
                ]
            )
        },
    )
    scenario.build(LIFECYCLE_SELECTOR)

    rows = scenario.warehouse.rows(events_sql, {"src": SOURCE_ID})

    assert sorted(
        (
            r["tenant_id"],
            r["issue_id"],
            r["id_readable"],
            r["entity_id"],
            r["author_id"],
            r["delta_action"],
            r["same_instant_rank"],
        )
        for r in rows
    ) == [
        (TENANT_ID, "1001", "TST-1", "501", "author-mine", "add", 0),
        (TENANT_ID, "2002", "TST-2", "503", "author-mine", "add", 0),
        (OTHER_TENANT, "2002", "OTH-7", "501", "author-theirs", "add", 0),
        (OTHER_TENANT, "2002", "OTH-7", "502", "author-theirs", "add", 1),
    ], f"should keep each tenant's {entity} events apart"
