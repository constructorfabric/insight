"""The relations the `on-run-start` hooks create follow the install's topology.

Three hooks registered in `dbt_project.yml` create the operator-authored `config`
relations and `identity.identity_persons` by handing raw DDL to `run_query`. That
bypasses the adapter, so neither of the two things the adapter does for a model
reaches them: the engine never gains its `Replicated` prefix, and the profile's
`cluster:` key never appends an `ON CLUSTER` clause. A hand-spelled engine here
would create a local table on one replica of a cluster -- the failure epic #2010
exists to remove -- while every model around it replicated correctly.

Both renderers live in `dbt/macros/insight_engine.sql`, and the source rules below
are what keeps the hooks using them. The warehouse rules then read what a dbt run
actually created, which is what a broken interpolation would show up in.

Scope of the warehouse rules is `config` alone. `identity.identity_persons` has a
second creator -- the `connectors-ddl` snapshot this suite applies before dbt runs
-- so its shape on a stand is not evidence about this hook until T23 retires that
snapshot. The source rules cover it either way.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest
from insight_datapath import clickhouse as ch
from insight_datapath.dbt_runner import DbtRunner
from insight_datapath.instance import InstanceConfig
from insight_datapath.topology import Topology, from_environment

REPO_ROOT = Path(__file__).resolve().parents[3]

#: The `on-run-start` hooks that emit DDL. `drop_silver_placeholders_at_start`
#: creates nothing and is deliberately not one of them.
RAW_DDL_MACROS = sorted((REPO_ROOT / "src/ingestion/dbt/macros").glob("create_*.sql"))

#: Any member of the MergeTree family, spelled out.
MERGE_TREE_FAMILY = re.compile(r"\b\w*MergeTree\b")

#: The DDL keyword a table's engine follows.
ENGINE_CLAUSE = re.compile(r"\bENGINE\s*=")

CREATES_A_RELATION = re.compile(r"\bCREATE (?:TABLE|DATABASE)\b")

#: The names a macro interpolates into its DDL, and the renderer each is bound to.
RENDERERS = {"engine": "insight_engine(", "on_cluster": "insight_on_cluster()"}
CLUSTER_CLAUSE_NAME = "on_cluster"

CONFIG_DATABASE = "config"

#: Every relation the hooks alone create, and the family each is created with.
CONFIG_RELATIONS = {
    "ai_credit_pricing": "ReplacingMergeTree",
    "ai_seat_tier_map": "ReplacingMergeTree",
    "field_value_defaults": "ReplacingMergeTree",
    "field_value_map": "ReplacingMergeTree",
    "task_field_roles": "ReplacingMergeTree",
    "task_value_map": "ReplacingMergeTree",
}


def _lines(macro: Path) -> list[tuple[int, str]]:
    return list(enumerate(macro.read_text(encoding="utf-8").splitlines(), start=1))


@pytest.fixture(scope="session")
def topology() -> Topology:
    """The topology this instance's creators were configured with."""
    return from_environment()


@pytest.fixture(scope="session")
def config_engines(instance_cfg: InstanceConfig, dbt_runner: DbtRunner) -> dict[str, str]:
    """The engine family ClickHouse reports for each relation the hooks created.

    Taken after a dbt run rather than after the schema fixture: `on-run-start` is
    the only thing that creates these, so without one there would be nothing here
    and every rule below would pass on an empty warehouse.
    """
    rows = ch.query(
        instance_cfg,
        f"SELECT name, engine FROM system.tables WHERE database = '{CONFIG_DATABASE}'",
    )
    return {str(name): str(engine) for name, engine in rows}


def test_the_hooks_are_the_macros_this_file_reads() -> None:
    """A rename would otherwise leave every source rule below passing vacuously."""
    assert [macro.name for macro in RAW_DDL_MACROS] == [
        "create_ai_config_tables.sql",
        "create_identity_persons.sql",
        "create_task_config_tables.sql",
    ]


@pytest.mark.parametrize("macro", RAW_DDL_MACROS, ids=lambda macro: macro.name)
def test_a_hook_never_spells_a_merge_tree_family_in_its_ddl(macro: Path) -> None:
    """A literal family is invisible to the topology: it stays plain on a cluster,
    where a plain tree replicates nothing and lives on the one replica that ran it."""
    literals = [
        f"{macro.relative_to(REPO_ROOT)}:{number}"
        for number, line in _lines(macro)
        if ENGINE_CLAUSE.search(line) and MERGE_TREE_FAMILY.search(line)
    ]

    assert not literals, f"engine spelled into the DDL rather than interpolated: {literals}"


@pytest.mark.parametrize("macro", RAW_DDL_MACROS, ids=lambda macro: macro.name)
def test_every_relation_a_hook_creates_is_qualified_by_the_cluster_clause(macro: Path) -> None:
    """Unqualified DDL reaches only the node that answered the connection, so the
    replicas a cluster-mode engine expects never learn the relation exists."""
    unqualified = [
        f"{macro.relative_to(REPO_ROOT)}:{number}"
        for number, line in _lines(macro)
        if CREATES_A_RELATION.search(line) and CLUSTER_CLAUSE_NAME not in line
    ]

    assert not unqualified, f"created without the ON CLUSTER clause: {unqualified}"


@pytest.mark.parametrize("macro", RAW_DDL_MACROS, ids=lambda macro: macro.name)
def test_what_a_hook_interpolates_comes_from_the_shared_renderers(macro: Path) -> None:
    """The two rules above only check that a name is interpolated; this is what makes
    the name mean what the rest of the release renders for the same topology."""
    source = macro.read_text(encoding="utf-8")
    rebound = [name for name, renderer in RENDERERS.items() if f"{name} = {renderer}" not in source]

    assert not rebound, f"{macro.name} binds {rebound} to something other than its renderer"


def test_a_dbt_run_creates_every_operator_authored_relation(config_engines: dict[str, str]) -> None:
    """A model reads an operator's bindings as a source, so a hook that failed to
    create one fails the build rather than reporting an unmapped value."""
    assert set(config_engines) == set(CONFIG_RELATIONS)


def test_an_operator_authored_relation_carries_the_family_its_topology_calls_for(
    config_engines: dict[str, str], topology: Topology
) -> None:
    """Read as the family name alone: a `Replicated*` engine also names its keeper
    path and replica, and both are an operator's choice no test may pin."""
    prefix = "Replicated" if topology is Topology.REPLICATED else ""
    expected = {name: f"{prefix}{family}" for name, family in CONFIG_RELATIONS.items()}

    assert config_engines == expected
