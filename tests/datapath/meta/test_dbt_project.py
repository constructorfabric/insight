"""What the dbt project must be true of for a spec's data to land where tests read it.

These read the manifest the session's runner already parsed, so they cost a lookup
rather than a build. Two of them are product contracts rather than rig ones: a
connector model no deploy pass selects is never materialized on a real sync, and the
identity map ships as a stale placeholder unless the deploy selects it.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest
from insight_datapath.dbt_runner import PROFILE_SCHEMA, DbtError, DbtRunner

REPO_ROOT = Path(__file__).resolve().parents[3]
MIGRATIONS_SCRIPT = REPO_ROOT / "src/ingestion/scripts/apply-ch-migrations.sh"

MAP_TAG = "identity:map"
MAP_MODELS = {"account_assignment", "person_map"}

#: The three tag-scoped passes the jira pipeline is built by; it has no catch-all.
JIRA_DEPLOY_PASSES = ("tag:staging,tag:jira", "tag:silver,tag:jira+", "tag:gold,tag:jira+")


@pytest.fixture(scope="module")
def manifest(dbt_runner: DbtRunner) -> dict[str, Any]:
    return json.loads((dbt_runner.target_dir / "manifest.json").read_text(encoding="utf-8"))


def _models(manifest: dict[str, Any]) -> dict[str, dict[str, Any]]:
    return {
        uid: node for uid, node in manifest["nodes"].items() if node.get("resource_type") == "model"
    }


def _tags(node: dict[str, Any]) -> set[str]:
    return set(node.get("config", {}).get("tags") or [])


def _selected(manifest: dict[str, Any], expression: str) -> set[str]:
    """`tag:X` atoms, `+` for descendants, comma for AND, space for OR — dbt's own rules."""
    nodes = _models(manifest)
    child_map = manifest.get("child_map", {})

    def descendants(seed: set[str]) -> set[str]:
        seen: set[str] = set()
        stack = list(seed)
        while stack:
            for child in child_map.get(stack.pop(), []):
                if child not in seen:
                    seen.add(child)
                    stack.append(child)
        return seen

    union: set[str] = set()
    for term in expression.split():
        intersection: set[str] | None = None
        for atom in term.split(","):
            plus = atom.endswith("+")
            tag = (atom[:-1] if plus else atom).removeprefix("tag:")
            base = {uid for uid, node in nodes.items() if tag in _tags(node)}
            reached = base | descendants(base) if plus else base
            intersection = reached if intersection is None else intersection & reached
        union |= intersection or set()
    return union


def test_a_relation_is_named_by_the_schema_dbt_writes_it_to(dbt_runner: DbtRunner) -> None:
    """A connector staging model lands in `staging` and a class model in `silver`, not
    in the profile's fallback schema the truncate ledger would otherwise register."""
    assert dbt_runner.materialized_relations(["github__commits", "class_git_commits"]) == [
        ("silver", "class_git_commits"),
        ("staging", "github__commits"),
    ]


def test_a_model_holding_no_rows_never_reaches_the_ledger(dbt_runner: DbtRunner) -> None:
    """A view and an ephemeral model have nothing to truncate."""
    assert (
        dbt_runner.materialized_relations(["github__bronze_promoted", "jira__task_field_history"])
        == []
    )


def test_a_name_no_model_answers_to_is_refused(dbt_runner: DbtRunner) -> None:
    """Recording nothing silently is what leaves a spec's rows behind for the next one."""
    with pytest.raises(DbtError, match="no dbt model is named"):
        dbt_runner.materialized_relations(["class_git_commits", "not_a_model"])


def test_every_materialized_model_declares_its_schema(manifest: dict[str, Any]) -> None:
    """A model configuring no schema lands in the fallback, where nothing looks for it:
    the ledger registers it there while the project reads `staging` or `silver`, and its
    rows survive every reset."""
    undeclared = sorted(
        node["name"]
        for node in _models(manifest).values()
        if node.get("config", {}).get("materialized") not in ("ephemeral", "view")
        and node.get("schema") == PROFILE_SCHEMA
    )
    assert undeclared == [], (
        f"these models materialize into the fallback schema {PROFILE_SCHEMA!r}: {undeclared}"
    )


def test_the_jira_promotion_is_selected_by_the_deploy_staging_pass(
    manifest: dict[str, Any],
) -> None:
    """The promotion flips bronze to ReplacingMergeTree, and the enrich step that runs
    next reads it FINAL. Tagged `jira` alone it matches no pass, and every real sync
    fails on a storage that does not support FINAL."""
    selected = {
        manifest["nodes"][uid]["name"] for uid in _selected(manifest, "tag:staging,tag:jira")
    }
    assert "jira__bronze_promoted" in selected


def test_every_jira_model_is_built_by_some_deploy_pass(manifest: dict[str, Any]) -> None:
    """A jira model no pass selects is never materialized on a real sync, and its silver
    consumers fail on a table that does not exist. Ephemeral models are inlined by their
    consumers, so no pass builds them standalone."""
    covered: set[str] = set()
    for expression in JIRA_DEPLOY_PASSES:
        covered |= _selected(manifest, expression)
    gaps = sorted(
        node["name"]
        for uid, node in _models(manifest).items()
        if "/connectors/task-tracking/jira/dbt/" in node.get("original_file_path", "")
        and node.get("config", {}).get("materialized") != "ephemeral"
        and uid not in covered
    )
    assert gaps == [], f"built by no deploy pass ({', '.join(JIRA_DEPLOY_PASSES)}): {gaps}"


def test_the_identity_map_models_carry_the_tag_the_deploy_selects(
    manifest: dict[str, Any],
) -> None:
    """An untagged map model ships as an empty placeholder from the DDL snapshot, and
    every person read resolves against it without erroring."""
    tagged = {node["name"] for node in _models(manifest).values() if MAP_TAG in _tags(node)}
    assert tagged == MAP_MODELS


def test_the_person_map_is_a_view_in_the_identity_schema(manifest: dict[str, Any]) -> None:
    person_map = next(node for node in _models(manifest).values() if node["name"] == "person_map")
    assert person_map["schema"] == "identity"
    assert person_map["config"]["materialized"] == "view"


def test_the_migrations_script_appends_the_map_tag_after_reading_any_override() -> None:
    """A caller passing DBT_GOLD_SELECT must not be able to drop the map from the build."""
    script = MIGRATIONS_SCRIPT.read_text(encoding="utf-8")
    override = '_dbt_select <<<"${DBT_GOLD_SELECT:-'
    append = '_dbt_select+=("tag:identity:map")'
    assert override in script, "the gold selector line moved; keep this contract with it"
    assert append in script, "the map tag is never appended, so an override leaves it stale"
    assert script.index(override) < script.index(append), (
        "the append must follow the override read, or an override drops the map"
    )
