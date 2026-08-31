"""The identity map must be built by every path that builds gold.

The DDL snapshot carries a point-in-time copy of the map, so a deploy that
skips it leaves every person read resolving against a stale relation and
nothing errors. So the selection is asserted rather than trusted.
"""

from __future__ import annotations

import json
import re
import subprocess
from dataclasses import dataclass
from functools import cache
from pathlib import Path

MAP_TAG = "identity:map"
MAP_MODELS = {"account_assignment", "person_map"}

INGESTION = Path(__file__).resolve().parents[3]
DBT_DIR = INGESTION / "dbt"
MIGRATIONS_SCRIPT = INGESTION / "scripts" / "apply-ch-migrations.sh"


@dataclass(frozen=True)
class ModelNode:
    """The manifest fields this contract reads, parsed at the boundary so a
    changed manifest shape fails here rather than inside an assertion."""

    name: str
    schema: str
    materialized: str
    tags: frozenset[str]


@cache
def _models() -> tuple[ModelNode, ...]:
    """Every model in the parsed manifest. Cached: `dbt parse` is a subprocess,
    and each test would otherwise pay for its own."""
    subprocess.run(
        ["dbt", "parse", "--profiles-dir", ".", "--target", "local"],
        cwd=DBT_DIR,
        check=True,
        capture_output=True,
    )
    manifest = json.loads((DBT_DIR / "target" / "manifest.json").read_text())
    return tuple(
        ModelNode(
            name=node["name"],
            schema=node["schema"],
            materialized=node["config"]["materialized"],
            tags=frozenset(node["config"]["tags"]),
        )
        for node in manifest["nodes"].values()
        if node["resource_type"] == "model"
    )


def test_the_map_models_carry_the_deploy_selected_tag() -> None:
    tagged = {model.name for model in _models() if MAP_TAG in model.tags}

    assert tagged == MAP_MODELS, (
        f"models tagged {MAP_TAG!r} are {sorted(tagged)}; an untagged map model "
        f"ships as an empty placeholder"
    )


def test_every_person_read_resolves_through_a_deploy_selected_model() -> None:
    map_model = next(model for model in _models() if model.name == "person_map")

    assert map_model.schema == "identity"
    assert map_model.materialized == "view"
    assert MAP_TAG in map_model.tags


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
