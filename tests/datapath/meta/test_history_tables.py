"""Every relation a full refresh cannot reproduce is tagged, and the tenant migration knows it.

A snapshot or an append-only (`full_refresh=false`) model keeps rows that exist
nowhere else. The tenant migration rewrites their tenant column in place and its
full refresh excludes `tag:history`, so a model of that kind without the tag
would lose its history, and a tagged model missing from tenant-history-tables.txt
would keep its legacy tenant.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest
from insight_datapath.dbt_runner import DbtRunner

REPO_ROOT = Path(__file__).resolve().parents[3]
HISTORY_TABLES = REPO_ROOT / "src/ingestion/scripts/tenant-history-tables.txt"
TAG = "history"
SNAPSHOT_MACRO = "macro.ingestion.snapshot"


@pytest.fixture(scope="module")
def models(dbt_runner: DbtRunner) -> dict[str, dict[str, Any]]:
    manifest = json.loads((dbt_runner.target_dir / "manifest.json").read_text(encoding="utf-8"))
    return {uid: n for uid, n in manifest["nodes"].items() if n.get("resource_type") == "model"}


def _keeps_history(node: dict[str, Any]) -> bool:
    return node["config"].get("full_refresh") is False or SNAPSHOT_MACRO in node["depends_on"].get("macros", [])


def test_every_model_keeping_history_is_tagged(models: dict[str, dict[str, Any]]) -> None:
    untagged = sorted(n["name"] for n in models.values() if _keeps_history(n) and TAG not in n["config"]["tags"])
    assert not untagged, f"add tags=['{TAG}'] to: {untagged}"


def test_the_migration_lists_exactly_the_tagged_relations(models: dict[str, dict[str, Any]]) -> None:
    tagged = {f"{n['schema']}.{n.get('alias') or n['name']}" for n in models.values() if TAG in n["config"]["tags"]}
    listed = {
        line.strip()
        for line in HISTORY_TABLES.read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    }
    assert listed == tagged, {"missing": sorted(tagged - listed), "stale": sorted(listed - tagged)}
