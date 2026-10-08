import json

import pytest
from insight_datapath.dbt_graph import from_manifest
from insight_datapath.dbt_runner import DbtRunner
from insight_datapath.leg_selection import Partial, Reach, reach, verdict_for
from insight_datapath.select_legs import REPO_ROOT
from insight_datapath.suite_scan import scan_suites

M365_ACTIVITY = "src/ingestion/connectors/collaboration/m365/dbt/m365__collab_email_activity.sql"


@pytest.fixture(scope="module")
def project(dbt_runner: DbtRunner) -> Reach:
    manifest = json.loads((dbt_runner.target_dir / "manifest.json").read_text(encoding="utf-8"))
    return reach(from_manifest(manifest), scan_suites(REPO_ROOT))


def test_every_relation_a_suite_seeds_is_read_by_a_dbt_source(project: Reach) -> None:
    """A seeded table no source reads is one the flow cannot follow, so its changes run no leg."""
    unread = {
        suite: sorted(table for table in tables if table not in project.graph.sources_by_relation)
        for suite, tables in project.suites.seeds.items()
    }
    assert not any(unread.values()), f"should be dbt sources: {unread}"


def test_an_m365_activity_model_runs_collab_and_never_git(project: Reach) -> None:
    verdict = verdict_for(M365_ACTIVITY, project)
    assert isinstance(verdict, Partial), f"should place {M365_ACTIVITY}, got {verdict}"
    assert "collab" in verdict.suites
    assert "git" not in verdict.suites
