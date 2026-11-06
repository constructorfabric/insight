"""A censused issue missing its full record is a finding only when it changed before the scan.

The census sees every issue, including those untouched since the start date that
the incremental scan legitimately never fetches. What separates a missed issue
from one of those is evidence the issue changed inside the scanned window: the
scan enumerated it, a worklog on it changed, or its id says it was created
inside the recent window, more than the race window before the scan last ran.

The id-floor arm measures from the first census sighting, which the
availability snapshot stamps with the build time, so every time here derives
from one instant captured before the build.
"""

from __future__ import annotations

import uuid
from dataclasses import dataclass
from datetime import UTC, datetime, timedelta
from typing import Any

from conftest import Scenario
from helpers import SOURCE_ID, TENANT_ID, census_entry, issue, worklog

SELECTOR = "+jira__issue_availability_history"
CHECK = "assert_census_issue_has_full_record"

FLOOR_ISSUE_ID = "20001"


@dataclass(frozen=True)
class CensusCase:
    rule: str
    key: str
    jira_id: str
    has_full_record: bool = False
    created_recently: bool = False
    enumerated_at: str | None = None
    worklog_changed_at: str | None = None
    flagged: bool = False


@dataclass(frozen=True)
class Clock:
    now: datetime

    @property
    def scan(self) -> datetime:
        return self.now + timedelta(minutes=30)

    @property
    def before_scan(self) -> str:
        return _jira_time(self.now - timedelta(days=10))

    @property
    def after_scan(self) -> str:
        return _jira_time(self.scan + timedelta(hours=1))


def _jira_time(at: datetime) -> str:
    return at.strftime("%Y-%m-%dT%H:%M:%S.000+0000")


def _stamp(at: datetime) -> str:
    return at.strftime("%Y-%m-%dT%H:%M:%S")


def _cases(clock: Clock) -> list[CensusCase]:
    return [
        CensusCase(
            "an issue with its full record is complete",
            "TST-1",
            "10001",
            has_full_record=True,
            enumerated_at=clock.before_scan,
        ),
        CensusCase(
            "an issue the scan enumerated but never fetched is missed",
            "TST-2",
            "10002",
            enumerated_at=clock.before_scan,
            flagged=True,
        ),
        CensusCase(
            "an issue whose worklog changed before the scan is missed",
            "TST-3",
            "10003",
            worklog_changed_at=clock.before_scan,
            flagged=True,
        ),
        CensusCase("an issue with no change evidence was legitimately never scanned", "TST-4", "10004"),
        CensusCase(
            "a change after the scan ran is not yet the scan's to collect",
            "TST-5",
            "10005",
            enumerated_at=clock.after_scan,
            worklog_changed_at=clock.after_scan,
        ),
        CensusCase(
            "a recently created issue sets the id floor",
            "TST-6",
            FLOOR_ISSUE_ID,
            has_full_record=True,
            created_recently=True,
        ),
        CensusCase(
            "an issue above the id floor first censused inside the race window is not yet missed", "TST-7", "20002"
        ),
    ]


def _airbyte(extracted_at: str) -> dict[str, Any]:
    return {
        "_airbyte_raw_id": str(uuid.uuid4()),
        "_airbyte_extracted_at": extracted_at,
        "_airbyte_meta": "{}",
        "_airbyte_generation_id": 0,
    }


def _issue_key(case: CensusCase, updated: str, extracted_at: str) -> dict[str, Any]:
    return {
        **_airbyte(extracted_at),
        "id": case.jira_id,
        "key": case.key,
        "tenant_id": TENANT_ID,
        "source_id": SOURCE_ID,
        "unique_key": f"{TENANT_ID}-{SOURCE_ID}-{case.jira_id}",
        "id_readable": case.key,
        "jira_id": case.jira_id,
        "updated": updated,
        "collected_at": extracted_at,
    }


def _flagged(scenario: Scenario) -> set[str]:
    invocation = scenario.warehouse.dbt_status("test", "--select", CHECK)
    assert "error" not in invocation.statuses, invocation.log
    rows = scenario.warehouse.rows(
        "SELECT jira_id FROM dbt_test__audit.assert_census_issue_has_full_record WHERE source_id = {source:String}",
        {"source": SOURCE_ID},
    )
    return {row["jira_id"] for row in rows}


def _full_record(case: CensusCase, clock: Clock) -> dict[str, Any]:
    created = _stamp(clock.now - timedelta(days=1)) if case.created_recently else _stamp(clock.now - timedelta(days=90))
    return issue(case.key, jira_id=case.jira_id, created=created, extracted_at=_stamp(clock.scan))


def test_a_missing_full_record_is_a_finding_only_for_an_issue_that_changed_before_the_scan(scenario: Scenario) -> None:
    clock = Clock(datetime.now(UTC).replace(microsecond=0))
    seen = _stamp(clock.now)
    cases = _cases(clock)

    scenario.seed(
        fields=[],
        issues=[_full_record(case, clock) for case in cases if case.has_full_record],
        census=[census_entry(case.key, jira_id=case.jira_id, extracted_at=seen) for case in cases],
    )
    scenario.warehouse.insert(
        "bronze_jira.jira_issue_keys",
        [_issue_key(case, case.enumerated_at, seen) for case in cases if case.enumerated_at],
    )
    scenario.warehouse.insert(
        "bronze_jira.jira_worklogs",
        [
            worklog(
                int(f"9{case.jira_id}"),
                jira_id=case.jira_id,
                key=case.key,
                started=case.worklog_changed_at,
                extracted_at=seen,
            )
            for case in cases
            if case.worklog_changed_at
        ],
    )

    scenario.build(SELECTOR)
    flagged = _flagged(scenario)

    for case in cases:
        assert (case.jira_id in flagged) == case.flagged, f"{case.rule}: {case.key} flagged={flagged}"
