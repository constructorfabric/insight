"""Every relation a full refresh cannot reproduce is tagged `history`, and listed.

A snapshot, a `full_refresh=false` model or a model that writes a value once and
keeps it holds rows that exist nowhere else: rebuilding it from what Bronze holds
today loses them. The tag is what keeps such a model out of a rebuild, and
`tenant-history-tables.txt` is how tooling outside dbt finds the same relations.

An append-only model is where such rows hide, and nothing in its SQL tells the two
kinds apart — so each one is decided by name: tagged `history`, or listed below as
rebuildable from its inputs.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

import pytest
from insight_datapath.dbt_parse import REPO_ROOT, Manifest, parse_project

HISTORY_TABLES = REPO_ROOT / "src/ingestion/scripts/tenant-history-tables.txt"
TAG = "history"
SNAPSHOT_MACRO = "macro.ingestion.snapshot"

#: Append-only models whose every row a rebuild derives again from Bronze or from a
#: `history` relation above them.
REBUILDABLE_APPEND_MODELS = frozenset(
    {
        "active_directory__identity_inputs",
        "active_directory__manager_identity_inputs",
        "active_directory__person_attribute_claims",
        "allure__categories",
        "allure__category_matchers",
        "allure__defect_test_results",
        "allure__defects",
        "allure__launch_environment",
        "allure__launch_errors",
        "allure__launches",
        "allure__test_cases",
        "allure__test_results",
        "bamboohr__identity_inputs",
        "bamboohr__person_attribute_claims",
        "bitbucket_cloud__identity_inputs",
        "chatgpt_team__ai_assistant_usage",
        "chatgpt_team__ai_credit_usage",
        "chatgpt_team__ai_dev_usage",
        "chatgpt_team__identity_inputs",
        "claude_enterprise__ai_assistant_usage",
        "claude_enterprise__ai_dev_usage",
        "claude_team__ai_dev_usage",
        "claude_team__identity_inputs",
        "cursor__ai_dev_usage",
        "github__identity_inputs",
        "github_directory__identity_inputs",
        "gitlab__identity_inputs",
        "hubspot__crm_accounts",
        "hubspot__crm_activities",
        "hubspot__crm_contacts",
        "hubspot__crm_deals",
        "hubspot__crm_users",
        "jira__availability_events",
        "jira__comment_lifecycle_events",
        "jira__identity_inputs",
        "jira__task_comments",
        "jira__task_field_metadata",
        "jira__task_worklogs",
        "jira__worklog_lifecycle_events",
        "ms_entra__identity_inputs",
        "ms_entra__person_attribute_claims",
        "outline__identity_inputs",
        "seed_identity_inputs_from_cursor",
        "zoom__collab_meeting_activity",
        "zoom__identity_inputs",
        "zoom__meeting_sessions",
        "zulip_proxy__identity_inputs",
    }
)

Models = dict[str, dict[str, Any]]


@pytest.fixture(scope="module")
def models(tmp_path_factory: pytest.TempPathFactory) -> Models:
    manifest: Manifest = parse_project(tmp_path_factory.mktemp("history"))
    return {n["name"]: n for n in manifest["nodes"].values() if n["resource_type"] == "model"}


def _tagged(node: dict[str, Any]) -> bool:
    return TAG in node["config"]["tags"]


def _appends(node: dict[str, Any]) -> bool:
    config = node["config"]
    return (
        config["materialized"] == "incremental" and config.get("incremental_strategy") == "append"
    )


def _keeps_history(node: dict[str, Any]) -> bool:
    """A snapshot, or an incremental model refusing a full refresh — a `table` model
    is rebuilt on every run whatever its `full_refresh` says."""
    config = node["config"]
    is_snapshot = SNAPSHOT_MACRO in node["depends_on"]["macros"]
    refuses = config.get("full_refresh") is False and config["materialized"] == "incremental"
    return refuses or is_snapshot


def _listed_relations(path: Path) -> set[str]:
    lines = (line.strip() for line in path.read_text(encoding="utf-8").splitlines())
    return {line for line in lines if line and not line.startswith("#")}


def test_a_snapshot_or_a_model_refusing_a_full_refresh_is_tagged(models: Models) -> None:
    untagged = sorted(name for name, n in models.items() if _keeps_history(n) and not _tagged(n))

    assert not untagged, f"add tags=['{TAG}'] to: {untagged}"


def test_every_append_only_model_is_decided(models: Models) -> None:
    """A new append-only model fails here until its author says which kind it is."""
    undecided = sorted(
        name
        for name, n in models.items()
        if _appends(n) and not _tagged(n) and name not in REBUILDABLE_APPEND_MODELS
    )

    assert not undecided, (
        f"tag as '{TAG}' if a rebuild cannot reproduce its rows, "
        f"else add to REBUILDABLE_APPEND_MODELS: {undecided}"
    )


def test_the_rebuildable_list_names_untagged_append_models(models: Models) -> None:
    stale = sorted(
        name
        for name in REBUILDABLE_APPEND_MODELS
        if name not in models or _tagged(models[name]) or not _appends(models[name])
    )

    assert not stale, f"no longer an untagged append-only model: {stale}"


def test_a_tagged_model_is_a_snapshot_or_an_incremental_model_refusing_a_full_refresh(
    models: Models,
) -> None:
    """Only a snapshot may be refreshed — the seed replaces an org that way."""
    exposed = sorted(name for name, n in models.items() if _tagged(n) and not _keeps_history(n))

    assert not exposed, f"make incremental with full_refresh=false: {exposed}"


def test_the_history_list_names_exactly_the_tagged_relations(models: Models) -> None:
    tagged = {f"{n['schema']}.{n.get('alias') or n['name']}" for n in models.values() if _tagged(n)}
    listed = _listed_relations(HISTORY_TABLES)

    assert listed == tagged, {"missing": sorted(tagged - listed), "stale": sorted(listed - tagged)}
