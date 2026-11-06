"""A censused issue missing its full record is a finding only when it changed before the scan.

The census sees every issue, including those untouched since the start date that
the incremental scan legitimately never fetches. What separates a missed issue
from one of those is evidence the issue changed inside the scanned window: the
scan enumerated it, or a worklog on it changed, before the scan last ran.
"""

from __future__ import annotations

import uuid
from dataclasses import dataclass
from typing import Any

from conftest import Scenario
from helpers import OBSERVED_AT, SOURCE_ID, TENANT_ID, census_entry, issue, worklog

SELECTOR = "+jira__issue_availability_history"
CHECK = "assert_census_issue_has_full_record"

BEFORE_SCAN = "2026-02-20T10:00:00.000+0000"
AFTER_SCAN = "2026-03-01T13:00:00.000+0000"


@dataclass(frozen=True)
class CensusCase:
    rule: str
    key: str
    jira_id: str
    has_full_record: bool = False
    enumerated_at: str | None = None
    worklog_changed_at: str | None = None
    flagged: bool = False


CASES = [
    CensusCase(
        "an issue with its full record is complete", "TST-1", "10001", has_full_record=True, enumerated_at=BEFORE_SCAN
    ),
    CensusCase(
        "an issue the scan enumerated but never fetched is missed",
        "TST-2",
        "10002",
        enumerated_at=BEFORE_SCAN,
        flagged=True,
    ),
    CensusCase(
        "an issue whose worklog changed before the scan is missed",
        "TST-3",
        "10003",
        worklog_changed_at=BEFORE_SCAN,
        flagged=True,
    ),
    CensusCase("an issue with no change evidence was legitimately never scanned", "TST-4", "10004"),
    CensusCase(
        "a change after the scan ran is not yet the scan's to collect",
        "TST-5",
        "10005",
        enumerated_at=AFTER_SCAN,
        worklog_changed_at=AFTER_SCAN,
    ),
]


def _airbyte() -> dict[str, Any]:
    return {
        "_airbyte_raw_id": str(uuid.uuid4()),
        "_airbyte_extracted_at": OBSERVED_AT,
        "_airbyte_meta": "{}",
        "_airbyte_generation_id": 0,
    }


def _issue_key(case: CensusCase, updated: str) -> dict[str, Any]:
    return {
        **_airbyte(),
        "id": case.jira_id,
        "key": case.key,
        "tenant_id": TENANT_ID,
        "source_id": SOURCE_ID,
        "unique_key": f"{TENANT_ID}-{SOURCE_ID}-{case.jira_id}",
        "id_readable": case.key,
        "jira_id": case.jira_id,
        "updated": updated,
        "collected_at": OBSERVED_AT,
    }


def _flagged(scenario: Scenario) -> set[str]:
    invocation = scenario.warehouse.dbt_status("test", "--select", CHECK)
    assert "error" not in invocation.statuses, invocation.log
    rows = scenario.warehouse.rows(
        "SELECT jira_id FROM dbt_test__audit.assert_census_issue_has_full_record WHERE source_id = {source:String}",
        {"source": SOURCE_ID},
    )
    return {row["jira_id"] for row in rows}


def test_a_missing_full_record_is_a_finding_only_for_an_issue_that_changed_before_the_scan(scenario: Scenario) -> None:
    scenario.seed(
        fields=[],
        issues=[issue(case.key, jira_id=case.jira_id) for case in CASES if case.has_full_record],
        census=[census_entry(case.key, jira_id=case.jira_id) for case in CASES],
    )
    scenario.warehouse.insert(
        "bronze_jira.jira_issue_keys", [_issue_key(case, case.enumerated_at) for case in CASES if case.enumerated_at]
    )
    scenario.warehouse.insert(
        "bronze_jira.jira_worklogs",
        [
            worklog(int(f"9{case.jira_id}"), jira_id=case.jira_id, key=case.key, started=case.worklog_changed_at)
            for case in CASES
            if case.worklog_changed_at
        ],
    )

    scenario.build(SELECTOR)
    flagged = _flagged(scenario)

    for case in CASES:
        assert (case.jira_id in flagged) == case.flagged, f"{case.rule}: {case.key} flagged={flagged}"
