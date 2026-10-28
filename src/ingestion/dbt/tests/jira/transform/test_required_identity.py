"""A Jira row without the identity its stream requires stops the transform.

Every Jira stream declares `tenant_id` and `source_id` required, and the
issue-level streams `jira_id`; bronze still holds them as `Nullable(String)`.
The models read them through `CAST(... AS String)`, so a NULL fails the run
instead of becoming an empty tenant, source or issue.
"""

from __future__ import annotations

import pytest
from conftest import Scenario
from helpers import issue

# Reads all three identity columns of `jira_issue` through CAST; `+` builds
# the staging models it joins.
SELECTOR = "+jira__issue_availability_state"
NULL_REJECTED = "Cannot convert NULL value to non-Nullable type"


@pytest.mark.parametrize("column", ["tenant_id", "source_id", "jira_id"])
def test_an_issue_row_missing_its_identity_fails_the_run(scenario: Scenario, column: str) -> None:
    scenario.seed(fields=[], issues=[{**issue("TST-1", jira_id="1001"), column: None}])

    attempt = scenario.warehouse.dbt_status("run", "--select", SELECTOR)

    assert not attempt.success, f"should fail on a NULL {column}"
    assert NULL_REJECTED in attempt.log, f"should name the NULL conversion for {column}:\n{attempt.log}"


def test_an_issue_row_with_its_identity_builds(scenario: Scenario) -> None:
    scenario.seed(fields=[], issues=[issue("TST-1", jira_id="1001")])

    attempt = scenario.warehouse.dbt_status("run", "--select", SELECTOR)

    assert attempt.success, attempt.log
