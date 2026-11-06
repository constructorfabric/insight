"""The user field history records a value appearing and a value being cleared.

`fields_history` diffs consecutive snapshot versions. A tracked column that is
NULL on one side must still diff against the value on the other, the way an
absent key does in `fields_history_raw`.
"""

from __future__ import annotations

from conftest import Scenario
from helpers import user

SELECTOR = "+jira__users_fields_history"
ACCOUNT = "acct-1"
EMAIL = "user@example.com"


def _snapshot_versions(scenario: Scenario, emails: list[str | None]) -> None:
    warehouse = scenario.warehouse
    warehouse.execute("TRUNCATE TABLE IF EXISTS bronze_jira.jira_user")
    warehouse.execute("TRUNCATE TABLE IF EXISTS staging.jira__users_snapshot")

    for day, email in enumerate(emails, start=1):
        warehouse.insert("bronze_jira.jira_user", [user(ACCOUNT, email=email, extracted_at=f"2026-01-0{day}T12:00:00")])
        warehouse.build(SELECTOR, full_refresh=False)


def _email_changes(scenario: Scenario) -> list[tuple[str, str]]:
    rows = scenario.warehouse.rows(
        "SELECT old_value, new_value FROM staging.jira__users_fields_history"
        " WHERE entity_id = {account:String} AND field_name = 'email' ORDER BY updated_at",
        {"account": ACCOUNT},
    )
    return [(r["old_value"], r["new_value"]) for r in rows]


def test_an_email_that_appears_and_is_cleared_records_both_changes(scenario: Scenario) -> None:
    _snapshot_versions(scenario, [None, EMAIL, None])

    assert _email_changes(scenario) == [("", EMAIL), (EMAIL, "")]


def test_an_email_present_from_the_first_version_records_its_arrival(scenario: Scenario) -> None:
    _snapshot_versions(scenario, [EMAIL])

    assert _email_changes(scenario) == [("", EMAIL)]
