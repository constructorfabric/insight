"""Logged hours stay with the tracker account that logged them.

Two accounts may share one email, and an account may be bound to a different
person, or to an excluded one, than the email resolves to. `logged_hours` is a
plain sum, so its evidence keeps every account's key on every day, even a day
both accounts of one email logged work on. The worklog-accuracy measures, which
gate worklog on in-progress time, still merge such a day into one person-day row.

Seeds `silver.class_task_worklogs` and `silver.class_task_users` directly: the
subject is what gold does with them, as in `test_assignee_attribution`.
"""

from __future__ import annotations

from typing import Any

import pytest
from conftest import Warehouse

SOURCE = "jira-logged-hours-test"
TENANT = "33333333-3333-3333-3333-333333333333"
SHARED_EMAIL = "shared@example.com"
DAY = "2026-02-03"

WORKLOGS_TABLE = "silver.class_task_worklogs"
USERS_TABLE = "silver.class_task_users"


def user(account: str, email: str | None) -> dict[str, Any]:
    return {
        "unique_key": f"{SOURCE}-{account}",
        "tenant_id": TENANT,
        "insight_source_id": SOURCE,
        "data_source": "jira",
        "user_id": account,
        "email": email,
        "display_name": account,
        "collected_at": f"{DAY} 09:00:00",
        "_version": 1,
    }


def worklog(worklog_id: str, author: str, seconds: int) -> dict[str, Any]:
    return {
        "unique_key": f"{SOURCE}-worklog-{worklog_id}",
        "insight_source_id": SOURCE,
        "data_source": "jira",
        "worklog_id": worklog_id,
        "id_readable": "LGH-1",
        "issue_id": "LGH-1",
        "author_id": author,
        "work_date": f"{DAY} 10:00:00",
        "duration_seconds": seconds,
        "collected_at": f"{DAY} 12:00:00",
        "is_deleted": 0,
        "_version": 1,
    }


@pytest.fixture
def logged_hours(warehouse: Warehouse) -> dict[str, float]:
    warehouse.build("+jira__task_worklogs")
    warehouse.build("class_task_worklogs")
    scope = {"src": SOURCE}
    warehouse.execute("DELETE FROM silver.class_task_worklogs WHERE insight_source_id = {src:String}", scope)
    warehouse.execute("DELETE FROM silver.class_task_users WHERE insight_source_id = {src:String}", scope)
    warehouse.execute("DELETE FROM silver.class_task_field_history WHERE insight_source_id = {src:String}", scope)
    warehouse.insert(USERS_TABLE, [user("first-acct", SHARED_EMAIL), user("second-acct", SHARED_EMAIL)])
    warehouse.insert(WORKLOGS_TABLE, [worklog("1", "first-acct", 3 * 3600), worklog("2", "second-acct", 3600)])

    warehouse.dbt(
        "run",
        "--select",
        "task_field_roles_current",
        "task_issue_state",
        "task_status_spans",
        "task_worklog_flow",
        "task_metric_evidence",
        "--full-refresh",
    )
    rows = warehouse.rows(
        """
        SELECT account_id, entity_id, sum(contribution) AS hours
        FROM insight.task_metric_evidence
        WHERE account_source_id = {src:String} AND measure_key = 'logged_hours'
        GROUP BY account_id, entity_id
        """,
        {"src": SOURCE},
    )
    return {f"{r['account_id']}|{r['entity_id']}": float(r["hours"]) for r in rows}


def test_each_account_keeps_its_own_logged_hours_on_a_shared_email_day(logged_hours: dict[str, float]) -> None:
    assert logged_hours == {f"first-acct|{SHARED_EMAIL}": 3.0, f"second-acct|{SHARED_EMAIL}": 1.0}, (
        f"logged_hours evidence by account: {logged_hours}"
    )
