"""Every model's engine is the one its install's topology needs.

A clustered ClickHouse creates `Replicated*` where a single node creates the plain
family (epic #2010). The models route their engine through the `insight_engine`
macro so that one decision reaches all of them; these parse the project under both
topologies and compare what each model would be created with.

Parsing never connects — the profile below is a placeholder — so this reads the
repository rather than a stand.
"""

from __future__ import annotations

import re
from typing import TYPE_CHECKING

import pytest
from insight_datapath.dbt_parse import DBT_PROJECT, REPO_ROOT, parse_project
from insight_datapath.topology import CLUSTER_MODE_VARIABLE

if TYPE_CHECKING:
    from collections.abc import Iterator
    from pathlib import Path

#: The model roots dbt_project.yml lists, plus the macros a model takes its config from.
MODEL_SOURCES = (
    REPO_ROOT / "src/ingestion/silver",
    REPO_ROOT / "src/ingestion/gold",
    REPO_ROOT / "src/ingestion/connectors",
    DBT_PROJECT,
)

#: The macros, whose hand-written DDL the adapter never sees.
MACROS = DBT_PROJECT / "macros"

#: dbt's own clean-targets, which hold compiled copies of everything above.
BUILD_DIRS = frozenset({"target", "dbt_packages"})

#: Any member of the MergeTree family spelled straight into an ENGINE clause.
ENGINE_LITERAL_IN_DDL = re.compile(r"ENGINE\s*=\s*\w*MergeTree", re.IGNORECASE)

#: A `config(engine=...)` that is not a call to the macro. The trailing `\S` is load
#: bearing: without something to match after it, `\s*` backtracks to zero width and the
#: lookahead then passes on the space it left behind, clearing a spaced-out literal.
ENGINE_LITERAL = re.compile(r"engine\s*=\s*(?!insight_engine\()\S")

Engines = dict[str, str]


def _model_sql() -> Iterator[Path]:
    for root in MODEL_SOURCES:
        for path in sorted(root.rglob("*.sql")):
            if BUILD_DIRS.isdisjoint(path.relative_to(root).parts):
                yield path


def _engines(root: Path, *, clustered: bool) -> Engines:
    """What each model that configures an engine would be created with."""
    workspace = root / ("clustered" if clustered else "standalone")
    with pytest.MonkeyPatch.context() as patch:
        patch.setenv(CLUSTER_MODE_VARIABLE, "true" if clustered else "false")
        # The flag reaches dbt through `env_var`, so a cache warmed under the other
        # topology would answer with the other topology's engines.
        manifest = parse_project(workspace)

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


def test_a_macro_states_its_engine_through_the_macro_rather_than_as_a_literal() -> None:
    """A macro's DDL goes through `run_query`, so the adapter's prefix never reaches
    it either; the family has to come from `insight_engine`."""
    literals = [
        f"{path.relative_to(REPO_ROOT)}:{number}"
        for path in sorted(MACROS.rglob("*.sql"))
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1)
        if ENGINE_LITERAL_IN_DDL.search(line)
    ]

    assert not literals, (
        f"engine spelled into DDL rather than rendered by insight_engine(): {literals}"
    )


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
