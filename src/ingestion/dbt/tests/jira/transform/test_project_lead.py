"""A Jira project carries its lead as the lead's account id, the same id every
other Jira person reference in the task classes carries.

Bronze gains `lead_account_id` on the connector's first sync that declares it;
until then the staging view still builds and reports no lead.
"""

from __future__ import annotations

import uuid
from collections.abc import Iterator
from dataclasses import dataclass
from typing import Any

import pytest
from conftest import Warehouse
from helpers import OBSERVED_AT, SOURCE_ID, TENANT_ID

SELECTOR = "jira__task_projects"


@dataclass(frozen=True)
class LeadCase:
    rule: str
    project_id: int
    lead_account_id: str | None
    lead_id: str | None


CASES = [
    LeadCase("a project's lead is its account id", 1, "acct-lead-0001", "acct-lead-0001"),
    LeadCase("a project without a lead has none", 2, None, None),
    LeadCase("an empty account id is no lead", 3, "", None),
]


def _project(project_id: int, lead_account_id: str | None) -> dict[str, Any]:
    return {
        "_airbyte_raw_id": str(uuid.uuid4()),
        "_airbyte_extracted_at": OBSERVED_AT,
        "_airbyte_meta": "{}",
        "_airbyte_generation_id": 0,
        "id": str(project_id),
        "tenant_id": TENANT_ID,
        "source_id": SOURCE_ID,
        "unique_key": f"{SOURCE_ID}-project-{project_id}",
        "project_id": project_id,
        "project_key": f"P{project_id}",
        "name": f"Project {project_id}",
        "lead_account_id": lead_account_id,
        "collected_at": OBSERVED_AT,
    }


def _without_lead(record: dict[str, Any]) -> dict[str, Any]:
    return {key: value for key, value in record.items() if key != "lead_account_id"}


def _leads(warehouse: Warehouse) -> dict[str, str | None]:
    rows = warehouse.rows(
        "SELECT project_id, lead_id FROM staging.jira__task_projects WHERE insight_source_id = {src:String}",
        {"src": SOURCE_ID},
    )
    return {row["project_id"]: row["lead_id"] for row in rows}


@pytest.fixture
def projects(warehouse: Warehouse) -> Iterator[Warehouse]:
    warehouse.execute("TRUNCATE TABLE IF EXISTS bronze_jira.jira_projects")
    yield warehouse
    warehouse.execute("TRUNCATE TABLE IF EXISTS bronze_jira.jira_projects")


def test_a_project_carries_its_leads_account_id(projects: Warehouse) -> None:
    projects.insert("bronze_jira.jira_projects", [_project(c.project_id, c.lead_account_id) for c in CASES])

    projects.build(SELECTOR)

    leads = _leads(projects)
    for case in CASES:
        assert leads.get(str(case.project_id)) == case.lead_id, f"{case.rule}: project {case.project_id}"


def test_bronze_synced_before_the_lead_column_still_builds(projects: Warehouse) -> None:
    projects.execute("ALTER TABLE bronze_jira.jira_projects DROP COLUMN lead_account_id")
    try:
        projects.insert("bronze_jira.jira_projects", [_without_lead(_project(1, "acct-lead-0001"))])

        projects.build(SELECTOR)

        assert _leads(projects) == {"1": None}, "a bronze table without the column should report no lead"
    finally:
        projects.execute(
            "ALTER TABLE bronze_jira.jira_projects ADD COLUMN IF NOT EXISTS lead_account_id Nullable(String) AFTER lead"
        )
