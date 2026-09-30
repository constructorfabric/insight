"""The one profiles.yml writer serves both dbt steps.

dbt-run opts into the correlated-subqueries profile setting; the data-quality
step does not — the flag is the only difference between the two profiles.

The adapter's `cluster` key is the other thing this file decides: present, it
turns every DDL statement dbt emits into an `ON CLUSTER` one, so it follows the
topology flag rather than a leftover name.

Run: pytest src/ingestion/scripts/tests
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

pytest.importorskip("yaml", reason="pyyaml rides the toolbox image via dbt-core")

from dbt_profiles import build_profile, on_cluster


@pytest.fixture(autouse=True)
def clickhouse_env(monkeypatch) -> None:
    monkeypatch.setenv("CLICKHOUSE_HOST", "clickhouse.example.internal")
    monkeypatch.setenv("CLICKHOUSE_PORT", "8123")
    monkeypatch.setenv("CLICKHOUSE_USER", "user-under-test")
    monkeypatch.setenv("CLICKHOUSE_PASSWORD", "password-under-test")
    monkeypatch.delenv("CLICKHOUSE_CLUSTER_MODE", raising=False)
    monkeypatch.delenv("CLICKHOUSE_CLUSTER_NAME", raising=False)


def test_the_flag_is_the_only_difference_between_the_two_steps() -> None:
    plain = build_profile(correlated_subqueries=False)["ingestion"]["outputs"]["k8s"]
    flagged = build_profile(correlated_subqueries=True)["ingestion"]["outputs"]["k8s"]

    assert "settings" not in plain
    assert flagged.pop("settings") == {"allow_experimental_correlated_subqueries": 1}
    assert flagged == plain


def test_the_profile_reads_the_connection_from_env() -> None:
    output = build_profile(correlated_subqueries=False)["ingestion"]["outputs"]["k8s"]

    assert output["host"] == "clickhouse.example.internal"
    assert output["port"] == 8123
    assert output["user"] == "user-under-test"
    assert output["password"] == "password-under-test"
    assert output["schema"] == "silver"


def _output() -> dict:
    return build_profile(correlated_subqueries=False)["ingestion"]["outputs"]["k8s"]


def test_a_standalone_install_writes_no_cluster_key() -> None:
    assert "cluster" not in _output()


@pytest.mark.parametrize("flag", ["1", "true", "TRUE", " yes ", "on"])
def test_a_clustered_install_names_its_cluster(monkeypatch, flag: str) -> None:
    monkeypatch.setenv("CLICKHOUSE_CLUSTER_MODE", flag)
    monkeypatch.setenv("CLICKHOUSE_CLUSTER_NAME", " insight_cluster ")

    assert on_cluster() == "insight_cluster", f"should read as clustered: {flag!r}"
    assert _output()["cluster"] == "insight_cluster"


def test_a_cluster_name_without_the_flag_names_no_cluster(monkeypatch) -> None:
    """The flag is what turns on the replicated engines an ON CLUSTER clause
    would qualify, so a leftover name must not half-enable clustering."""
    monkeypatch.setenv("CLICKHOUSE_CLUSTER_NAME", "insight_cluster")

    assert on_cluster() == ""
    assert "cluster" not in _output()


def test_a_replicated_database_needs_no_clause(monkeypatch) -> None:
    """The epic's chosen mechanism: the database engine distributes the DDL,
    so the flag stands alone and no clause is written."""
    monkeypatch.setenv("CLICKHOUSE_CLUSTER_MODE", "true")

    assert on_cluster() == ""
    assert "cluster" not in _output()
