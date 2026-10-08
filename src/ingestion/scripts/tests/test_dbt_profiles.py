"""The one profiles.yml writer serves every dbt step.

Each step picks a target name and a warehouse; everything else — the timeouts,
the session settings, the adapter's `cluster` key — comes from here, so a
topology decision reaches the deploy hook, bootstrap, the seeder and both test
rigs without being restated in any of them.

Run: pytest src/ingestion/scripts/tests
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

pytest.importorskip("yaml", reason="pyyaml rides the toolbox image via dbt-core")

from dbt_profiles import Connection, build_output, build_profile, connection_from_env, on_cluster

CONNECTION = Connection(
    host="clickhouse.example.internal", port=8123, user="user-under-test", password="password-under-test"
)


@pytest.fixture(autouse=True)
def clickhouse_env(monkeypatch) -> None:
    monkeypatch.setenv("CLICKHOUSE_HOST", "clickhouse.example.internal")
    monkeypatch.setenv("CLICKHOUSE_PORT", "8123")
    monkeypatch.setenv("CLICKHOUSE_USER", "user-under-test")
    monkeypatch.setenv("CLICKHOUSE_PASSWORD", "password-under-test")
    monkeypatch.delenv("CLICKHOUSE_URL", raising=False)
    monkeypatch.delenv("CLICKHOUSE_PROTOCOL", raising=False)
    monkeypatch.delenv("CLICKHOUSE_CLUSTER_MODE", raising=False)
    monkeypatch.delenv("CLICKHOUSE_CLUSTER_NAME", raising=False)


def _output(*, correlated_subqueries: bool = False) -> dict:
    return build_output(CONNECTION, correlated_subqueries=correlated_subqueries)


def test_the_flag_is_the_only_difference_between_the_two_steps() -> None:
    plain = _output()
    flagged = _output(correlated_subqueries=True)

    assert "custom_settings" not in plain
    assert flagged.pop("custom_settings") == {"allow_experimental_correlated_subqueries": 1}
    assert flagged == plain


def test_every_step_names_its_own_target() -> None:
    """A profile holds one output, under the name the step selects with --target."""
    profile = build_profile("migrate", CONNECTION, correlated_subqueries=False)

    assert profile["ingestion"]["target"] == "migrate"
    assert list(profile["ingestion"]["outputs"]) == ["migrate"]


def test_a_step_that_names_no_schema_materializes_into_silver() -> None:
    assert _output()["schema"] == "silver"


def test_a_rig_may_point_its_models_at_another_schema() -> None:
    """The data-path harness keeps un-schemaed models out of the relations a
    spec's reset clears."""
    output = build_output(
        Connection(host="h", port=8123, user="u", password="p", schema="default"), correlated_subqueries=False
    )

    assert output["schema"] == "default"


def test_a_session_setting_is_written_where_the_adapter_reads_it() -> None:
    """dbt-clickhouse takes connection settings from `custom_settings` alone — a
    plain `settings:` block is accepted and dropped, and `engine` with it."""
    output = _output(correlated_subqueries=True)

    assert "settings" not in output
    assert "engine" not in output
    assert output["custom_settings"] == {"allow_experimental_correlated_subqueries": 1}


def test_a_step_runs_dbt_single_threaded_unless_it_asks_for_more() -> None:
    assert _output()["threads"] == 1
    assert build_output(CONNECTION, correlated_subqueries=False, threads=2)["threads"] == 2


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


def test_a_flag_without_a_name_writes_no_clause(monkeypatch) -> None:
    """The chart refuses that pair, so this records only that the writer neither
    invents a cluster nor half-writes one if it is ever reached."""
    monkeypatch.setenv("CLICKHOUSE_CLUSTER_MODE", "true")

    assert on_cluster() == ""
    assert "cluster" not in _output()


def test_a_clustered_install_reads_what_the_step_before_it_wrote(monkeypatch) -> None:
    """Without a quorum on the write and sequential consistency on the read, a
    replica that has not caught up answers the next model with a partial table."""
    monkeypatch.setenv("CLICKHOUSE_CLUSTER_MODE", "true")

    assert _output()["custom_settings"] == {"insert_quorum": "auto", "select_sequential_consistency": 1}


def test_a_single_node_waits_for_no_quorum() -> None:
    assert "custom_settings" not in _output()


def test_the_connection_comes_from_host_and_port() -> None:
    connection = connection_from_env()

    assert connection == CONNECTION
    assert connection.secure is False


def test_a_step_that_speaks_https_connects_securely(monkeypatch) -> None:
    monkeypatch.setenv("CLICKHOUSE_PROTOCOL", "https")

    assert connection_from_env().secure is True


@pytest.mark.parametrize(
    ("url", "expected_port", "expected_secure"),
    [
        ("http://clickhouse.example.internal:8123", 8123, False),
        ("http://clickhouse.example.internal", 8123, False),
        ("https://clickhouse.example.internal", 8443, True),
        ("https://clickhouse.example.internal:9440", 9440, True),
    ],
)
def test_a_step_that_knows_only_a_url_reads_the_connection_out_of_it(
    monkeypatch, url: str, expected_port: int, expected_secure: bool
) -> None:
    """The deploy hook and the seeder carry `CLICKHOUSE_URL`, not a host/port pair."""
    monkeypatch.setenv("CLICKHOUSE_URL", url)

    connection = connection_from_env()

    assert connection.host == "clickhouse.example.internal", f"should parse: {url!r}"
    assert connection.port == expected_port, f"should parse: {url!r}"
    assert connection.secure is expected_secure, f"should parse: {url!r}"


def test_a_url_that_names_no_host_is_refused(monkeypatch) -> None:
    monkeypatch.setenv("CLICKHOUSE_URL", "clickhouse.example.internal:8123")

    with pytest.raises(ValueError, match="names no host"):
        connection_from_env()
