"""The identity map must be built by every path that builds gold.

The DDL snapshot carries a point-in-time copy of the map, so a deploy that
skips it leaves every person read resolving against a stale relation and
nothing errors. So the selection is asserted rather than trusted.
"""

from __future__ import annotations

import json
import re
import subprocess
from pathlib import Path

MAP_TAG = "identity:map"
MAP_MODELS = {"account_assignment", "person_map"}

INGESTION = Path(__file__).resolve().parents[3]
DBT_DIR = INGESTION / "dbt"
MIGRATIONS_SCRIPT = INGESTION / "scripts" / "apply-ch-migrations.sh"


def _manifest() -> dict:
    subprocess.run(
        ["dbt", "parse", "--profiles-dir", ".", "--target", "local"],
        cwd=DBT_DIR,
        check=True,
        capture_output=True,
    )
    return json.loads((DBT_DIR / "target" / "manifest.json").read_text())


def test_the_map_models_carry_the_deploy_selected_tag() -> None:
    nodes = _manifest()["nodes"].values()
    tagged = {
        node["name"]
        for node in nodes
        if node["resource_type"] == "model" and MAP_TAG in node["config"]["tags"]
    }

    assert tagged == MAP_MODELS, (
        f"models tagged {MAP_TAG!r} are {sorted(tagged)}; an untagged map model "
        f"never gets rebuilt from the snapshot's copy"
    )


def test_every_person_read_resolves_through_a_deploy_selected_model() -> None:
    nodes = _manifest()["nodes"].values()
    map_model = next(
        node
        for node in nodes
        if node["resource_type"] == "model" and node["name"] == "person_map"
    )

    assert map_model["schema"] == "identity"
    assert map_model["config"]["materialized"] == "view"
    assert MAP_TAG in map_model["config"]["tags"]


def test_the_migrations_script_appends_the_map_tag_after_any_override() -> None:
    script = MIGRATIONS_SCRIPT.read_text()

    override = re.search(r'_dbt_select <<<"\$\{DBT_GOLD_SELECT:-([^}]*)\}"', script)
    assert override, "the gold selector line moved; keep this contract with it"

    append = re.search(r'_dbt_select\+=\("tag:identity:map"\)', script)
    assert append, (
        "apply-ch-migrations.sh must append tag:identity:map to the selector, or "
        "a caller passing DBT_GOLD_SELECT leaves the map stale"
    )
    assert script.index(override.group(0)) < script.index(append.group(0)), (
        "the append must follow the override read, or an override drops the map"
    )
