"""An assigned issue keeps its row in `task_issue_state` whatever its assignee's profile says.

The table is the per-person attribution root, and the analytics runtime resolves
its rows account-first, so an issue needs only its assignee's account to stay:
an account with no email and one the user directory never listed both keep
their issues under the account. Only an issue with no assignee has no row.
An account id a second tenant also holds under the same source label reads
the directory of its own tenant only.

Seeds `silver.class_task_field_history` and `silver.class_task_users` directly:
the subject is what gold does with them, as in `test_title_role`.
"""

from __future__ import annotations

from typing import Any

import pytest
from conftest import Warehouse

SOURCE = "jira-assignee-attribution-test"
TENANT = "22222222-2222-2222-2222-222222222222"
OTHER_TENANT = "33333333-3333-3333-3333-333333333333"
EMAIL = "attribution@example.com"
OTHER_EMAIL = "other-tenant@example.com"

HISTORY_TABLE = "silver.class_task_field_history"
USERS_TABLE = "silver.class_task_users"


def history_row(issue: str, field_id: str, value: str | None, seq: int, tenant: str) -> dict[str, Any]:
    values = [] if value is None else [value]
    at = "2026-02-02 09:00:00"
    return {
        "unique_key": f"{tenant}-{issue}-{field_id}-{seq}",
        "tenant_id": tenant,
        "insight_source_id": SOURCE,
        "data_source": "jira",
        "issue_id": issue,
        "id_readable": issue,
        "event_id": f"initial:{issue}",
        "event_at": at,
        "event_kind": "synthetic_initial",
        "_seq": seq,
        "author_id": "actor",
        "field_id": field_id,
        "field_name": field_id,
        "field_cardinality": "single",
        "delta_action": "set",
        "value_ids": values,
        "value_displays": values,
        "value_id_type": "none",
        "collected_at": at,
        "_version": 1,
    }


def issue_assigned_to(issue: str, account: str | None, tenant: str = TENANT) -> list[dict[str, Any]]:
    return [history_row(issue, "created", None, 0, tenant), history_row(issue, "assignee", account, 1, tenant)]


def user(account: str, email: str | None, tenant: str = TENANT) -> dict[str, Any]:
    return {
        "unique_key": f"{tenant}-{SOURCE}-{account}",
        "tenant_id": tenant,
        "insight_source_id": SOURCE,
        "data_source": "jira",
        "user_id": account,
        "email": email,
        "display_name": account,
        "collected_at": "2026-02-02 09:00:00",
        "_version": 1,
    }


@pytest.fixture
def attribution(warehouse: Warehouse) -> dict[str, dict[str, Any]]:
    warehouse.execute(
        "DELETE FROM silver.class_task_field_history WHERE insight_source_id = {src:String}", {"src": SOURCE}
    )
    warehouse.execute("DELETE FROM silver.class_task_users WHERE insight_source_id = {src:String}", {"src": SOURCE})
    warehouse.insert(
        USERS_TABLE,
        [
            user("emailed-acct", EMAIL),
            user("emailless-acct", None),
            user("shared-acct", EMAIL),
            user("shared-acct", OTHER_EMAIL, OTHER_TENANT),
        ],
    )
    warehouse.insert(
        HISTORY_TABLE,
        [
            *issue_assigned_to("ATR-1", "emailed-acct"),
            *issue_assigned_to("ATR-2", "emailless-acct"),
            *issue_assigned_to("ATR-3", "unlisted-acct"),
            *issue_assigned_to("ATR-4", None),
            *issue_assigned_to("ATR-5", "shared-acct"),
            *issue_assigned_to("ATR-6", "shared-acct", OTHER_TENANT),
            *issue_assigned_to("ATR-7", "emailed-acct", OTHER_TENANT),
        ],
    )

    warehouse.dbt("run", "--select", "task_field_roles_current", "task_issue_state", "--full-refresh")
    rows = warehouse.rows(
        """
        SELECT issue_id, tenant_id, entity_id, account_source_type, account_source_id, account_id
        FROM insight.task_issue_state
        WHERE insight_source_id = {src:String}
        """,
        {"src": SOURCE},
    )
    return {str(r["issue_id"]): r for r in rows}


@pytest.mark.parametrize(
    ("issue", "entity_id", "account_id"),
    [("ATR-1", EMAIL, "emailed-acct"), ("ATR-2", "", "emailless-acct"), ("ATR-3", "", "unlisted-acct")],
)
def test_an_assigned_issue_keeps_its_row_under_the_account(
    attribution: dict[str, dict[str, Any]], issue: str, entity_id: str, account_id: str
) -> None:
    row = attribution.get(issue)
    assert row is not None, f"should keep a row: {issue}"
    assert (row["tenant_id"], row["entity_id"]) == (TENANT, entity_id), f"attribution of {issue}: {row}"
    assert (row["account_source_type"], row["account_source_id"], row["account_id"]) == ("jira", SOURCE, account_id), (
        f"account key of {issue}: {row}"
    )


def test_an_unassigned_issue_attributes_to_nobody(attribution: dict[str, dict[str, Any]]) -> None:
    assert "ATR-4" not in attribution, f"an unassigned issue has no row: {attribution.get('ATR-4')}"


@pytest.mark.parametrize(
    ("issue", "tenant", "entity_id"),
    [("ATR-5", TENANT, EMAIL), ("ATR-6", OTHER_TENANT, OTHER_EMAIL), ("ATR-7", OTHER_TENANT, "")],
)
def test_an_account_reads_the_directory_of_its_own_tenant_only(
    attribution: dict[str, dict[str, Any]], issue: str, tenant: str, entity_id: str
) -> None:
    row = attribution.get(issue)
    assert row is not None, f"should keep a row: {issue}"
    assert (row["tenant_id"], row["entity_id"]) == (tenant, entity_id), f"attribution of {issue}: {row}"
