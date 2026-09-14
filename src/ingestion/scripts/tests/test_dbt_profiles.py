"""The one profiles.yml writer serves both dbt steps.

dbt-run opts into the correlated-subqueries profile setting; the data-quality
step does not — the flag is the only difference between the two profiles.

Run: pytest src/ingestion/scripts/tests
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

pytest.importorskip("yaml", reason="pyyaml rides the toolbox image via dbt-core")

from dbt_profiles import build_profile


@pytest.fixture(autouse=True)
def clickhouse_env(monkeypatch) -> None:
    monkeypatch.setenv("CLICKHOUSE_HOST", "clickhouse.example.internal")
    monkeypatch.setenv("CLICKHOUSE_PORT", "8123")
    monkeypatch.setenv("CLICKHOUSE_USER", "user-under-test")
    monkeypatch.setenv("CLICKHOUSE_PASSWORD", "password-under-test")


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
