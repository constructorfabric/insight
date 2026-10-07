"""Every model's engine is the one its install's topology needs.

A clustered ClickHouse creates `Replicated*` where a single node creates the plain
family (epic #2010). The models route their engine through the `insight_engine`
macro so that one decision reaches all of them; these parse the project under both
topologies and compare what each model would be created with.

Parsing never connects — the profile below is a placeholder — so this reads the
repository rather than a stand.
"""

from __future__ import annotations

import json
import re
from pathlib import Path
from typing import TYPE_CHECKING

import pytest
import yaml

# dbt cannot share an environment with mypy — see the `datapath` dependency group.
from dbt.cli.main import dbtRunner  # type: ignore[import-not-found]
from insight_datapath.topology import CLUSTER_MODE_VARIABLE

if TYPE_CHECKING:
    from collections.abc import Iterator

REPO_ROOT = Path(__file__).resolve().parents[3]
DBT_PROJECT = REPO_ROOT / "src/ingestion/dbt"

#: The model roots dbt_project.yml lists, plus the macros a model takes its config from.
MODEL_SOURCES = (
    REPO_ROOT / "src/ingestion/silver",
    REPO_ROOT / "src/ingestion/gold",
    REPO_ROOT / "src/ingestion/connectors",
    DBT_PROJECT,
)

#: dbt's own clean-targets, which hold compiled copies of everything above.
BUILD_DIRS = frozenset({"target", "dbt_packages"})

#: A `config(engine=...)` that is not a call to the macro.
ENGINE_LITERAL = re.compile(r"engine\s*=\s*(?!insight_engine\()")

Engines = dict[str, str]


def _model_sql() -> Iterator[Path]:
    for root in MODEL_SOURCES:
        for path in sorted(root.rglob("*.sql")):
            if BUILD_DIRS.isdisjoint(path.relative_to(root).parts):
                yield path


def _write_profile(directory: Path) -> Path:
    """A profile dbt can load and will never connect with."""
    directory.mkdir(parents=True)
    profile = {
        "ingestion": {
            "target": "parse",
            "outputs": {
                "parse": {
                    "type": "clickhouse",
                    "host": "parse.invalid",
                    "port": 8123,
                    "schema": "parse",
                    "user": "parse",
                    "password": "parse",
                    "secure": False,
                }
            },
        }
    }
    (directory / "profiles.yml").write_text(yaml.safe_dump(profile), encoding="utf-8")
    return directory


def _engines(root: Path, *, clustered: bool) -> Engines:
    """What each model that configures an engine would be created with."""
    workspace = root / ("clustered" if clustered else "standalone")
    profiles_dir = _write_profile(workspace / "profiles")
    target_dir = workspace / "target"

    with pytest.MonkeyPatch.context() as patch:
        patch.setenv(CLUSTER_MODE_VARIABLE, "true" if clustered else "false")
        result = dbtRunner().invoke(
            [
                "parse",
                # The flag reaches dbt through `env_var`, so a cache warmed under
                # the other topology would answer with the other topology's engines.
                "--no-partial-parse",
                "--project-dir",
                str(DBT_PROJECT),
                "--profiles-dir",
                str(profiles_dir),
                "--target-path",
                str(target_dir),
            ]
        )

    if not result.success:
        raise AssertionError(f"dbt parse failed under clustered={clustered}: {result.exception}")

    manifest = json.loads((target_dir / "manifest.json").read_text(encoding="utf-8"))
    return {
        node["name"]: node["config"]["engine"]
        for node in manifest["nodes"].values()
        if node["resource_type"] == "model" and node["config"].get("engine")
    }


@pytest.fixture(scope="module")
def engines(tmp_path_factory: pytest.TempPathFactory) -> tuple[Engines, Engines]:
    """The project parsed once per topology: (single node, clustered)."""
    root = tmp_path_factory.mktemp("engines")
    return _engines(root, clustered=False), _engines(root, clustered=True)


def test_a_model_states_its_engine_through_the_macro_rather_than_as_a_literal() -> None:
    """A literal is invisible to the topology, so it would create a local table on one
    replica of a cluster — the failure this epic exists to remove."""
    literals = [
        f"{path.relative_to(REPO_ROOT)}:{number}"
        for path in _model_sql()
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1)
        if ENGINE_LITERAL.search(line)
    ]

    assert not literals, f"engine configured without insight_engine(): {literals}"


def test_a_single_node_is_created_with_the_plain_merge_tree_family(
    engines: tuple[Engines, Engines],
) -> None:
    standalone, _ = engines
    replicated = {name: engine for name, engine in standalone.items() if "Replicated" in engine}

    assert standalone, "no model configures an engine — the parse found nothing to check"
    assert not replicated, f"a single node cannot replicate: {replicated}"


def test_the_same_models_replicate_where_the_install_declares_a_cluster(
    engines: tuple[Engines, Engines],
) -> None:
    """Prefixed, and nothing else: the ordering key and the version column a model
    deduplicates on survive the switch, because only the family name changes."""
    standalone, clustered = engines
    expected = {name: f"Replicated{engine}" for name, engine in standalone.items()}

    assert clustered == expected
