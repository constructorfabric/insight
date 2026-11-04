"""The checks every Jira sync runs report each defect they exist to catch.

`connector-checks` runs `dbt test --select tag:connector_quality,tag:jira` after
a Jira transform. This module builds a clean dataset and asserts that selection
finds nothing, then plants one defect per completeness check and asserts each
is a finding of its own check.

The census check measures an issue's first sighting, which the availability
snapshot stamps with the build time, against the newest issue extraction. The
issue rows are therefore extracted two hours after the build, as if the issue
scan ran a sync after the census first saw them.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from datetime import UTC, datetime, timedelta

import pytest
from conftest import PROD_SELECTOR, Invocation, Warehouse, truncate_bronze
from helpers import census_entry, event, field, issue, item, worklog, worklog_tombstone

POST_SYNC_SELECTION = "tag:connector_quality,tag:jira"
SELECTOR = f"{PROD_SELECTOR} jira__issue_availability_history class_task_worklogs"

HEALTHY_ISSUE_ID = "10001"
UNFETCHED_ISSUE_ID = "10002"
LIVE_WORKLOG_ID = 5001
DELETED_WORKLOG_ID = 5002

CENSUS_CHECK = "assert_census_issue_has_full_record"
AVAILABILITY_CHECK = "assert_availability_ids_and_timestamps"
WORKLOG_CHECK = "assert_worklog_deletion_state_consistent"
DOUBLE_ENCODING_CHECK = "assert_changelog_items_not_double_encoded"
TAGGED_CHECKS = (CENSUS_CHECK, AVAILABILITY_CHECK, WORKLOG_CHECK)
FINDING_STATUSES = frozenset({"fail", "warn", "error"})


@dataclass(frozen=True)
class Runs:
    clean: Invocation
    planted: Invocation


def _stamp(at: datetime) -> str:
    return at.strftime("%Y-%m-%dT%H:%M:%S")


def _seed_clean(warehouse: Warehouse, now: datetime) -> None:
    """One censused issue with its full record, a live worklog, and a tombstoned
    one the pipeline marks deleted."""
    created = _stamp(now - timedelta(days=1))
    scanned = _stamp(now + timedelta(hours=2))
    seen = _stamp(now)

    warehouse.insert(
        "bronze_jira.jira_fields", [field("labels", name="Labels", schema_type="array", schema_items="string")]
    )
    warehouse.insert(
        "bronze_jira.jira_issue",
        [issue("TST-1", jira_id=HEALTHY_ISSUE_ID, created=created, extracted_at=scanned, fields={"labels": ["alpha"]})],
    )
    warehouse.insert(
        "bronze_jira.jira_issue_census", [census_entry("TST-1", jira_id=HEALTHY_ISSUE_ID, extracted_at=seen)]
    )
    warehouse.insert(
        "bronze_jira.jira_worklogs",
        [
            worklog(LIVE_WORKLOG_ID, jira_id=HEALTHY_ISSUE_ID, key="TST-1", started=created, extracted_at=seen),
            worklog(DELETED_WORKLOG_ID, jira_id=HEALTHY_ISSUE_ID, key="TST-1", started=created, extracted_at=seen),
        ],
    )
    warehouse.insert(
        "bronze_jira.jira_worklog_deleted",
        [worklog_tombstone(DELETED_WORKLOG_ID, deleted_at_ms=int(now.timestamp() * 1000), extracted_at=seen)],
    )


def _plant_defects_before_the_build(warehouse: Warehouse, now: datetime) -> None:
    seen = _stamp(now)
    changelog = event(
        "TST-1",
        101,
        _stamp(now - timedelta(hours=12)),
        [item("labels", to_str="alpha")],
        jira_id=HEALTHY_ISSUE_ID,
        extracted_at=seen,
    )

    warehouse.insert(
        "bronze_jira.jira_issue_census",
        [
            census_entry("TST-2", jira_id=UNFETCHED_ISSUE_ID, extracted_at=seen),
            census_entry("TST-3", jira_id="", extracted_at=seen),
        ],
    )
    warehouse.insert("bronze_jira.jira_issue_history", [{**changelog, "items": json.dumps(changelog["items"])}])


def _plant_defects_after_the_build(warehouse: Warehouse, now: datetime) -> None:
    """A tombstone the class contract has not caught up with: the worklog stays live."""
    warehouse.insert(
        "bronze_jira.jira_worklog_deleted",
        [worklog_tombstone(LIVE_WORKLOG_ID, deleted_at_ms=int(now.timestamp() * 1000), extracted_at=_stamp(now))],
    )


@pytest.fixture(scope="module")
def runs(warehouse: Warehouse) -> Runs:
    truncate_bronze(warehouse)
    now = datetime.now(UTC).replace(microsecond=0)

    _seed_clean(warehouse, now)
    warehouse.build(SELECTOR)
    clean = warehouse.dbt_status("test", "--select", POST_SYNC_SELECTION)

    _plant_defects_before_the_build(warehouse, now)
    warehouse.build(SELECTOR)
    _plant_defects_after_the_build(warehouse, now)
    planted = warehouse.dbt_status("test", "--select", POST_SYNC_SELECTION)

    return Runs(clean=clean, planted=planted)


def test_the_post_sync_selection_finds_nothing_on_clean_data(runs: Runs) -> None:
    findings = {name: status for name, status in runs.clean.status_by_node.items() if status in FINDING_STATUSES}

    assert set(TAGGED_CHECKS) <= runs.clean.status_by_node.keys(), (
        f"the selection should run every completeness check: {sorted(runs.clean.status_by_node)}"
    )
    assert findings == {}, f"should report nothing on clean data:\n{runs.clean.log}"


@pytest.mark.parametrize(
    "check",
    [
        pytest.param(CENSUS_CHECK, id="a censused issue with no full record"),
        pytest.param(AVAILABILITY_CHECK, id="an availability row without its id"),
        pytest.param(WORKLOG_CHECK, id="a tombstoned worklog still live"),
        pytest.param(
            DOUBLE_ENCODING_CHECK,
            id="a double-encoded changelog row",
            marks=pytest.mark.xfail(
                strict=True, reason="the check carries no catalog tag, so the post-sync step never runs it"
            ),
        ),
    ],
)
def test_a_planted_defect_is_a_finding_of_its_check(runs: Runs, check: str) -> None:
    assert runs.planted.status_by_node.get(check) == "fail", (
        f"{check} should report its planted defect; statuses: {runs.planted.status_by_node}"
    )
