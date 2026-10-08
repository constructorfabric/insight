"""Warm mode: which tenant a spec's Bronze is seeded under, and what runs over it."""

from __future__ import annotations

from pathlib import Path

import pytest
from insight_datapath.instance import InstanceConfig
from insight_datapath.tenant_transition import (
    WARM_KEY_ENV,
    TransitionError,
    WarmTenant,
    apply_transition,
    warm_tenant,
)

CANONICAL = "0f6b3c2e-5d4a-4e7b-9c1d-2a3b4c5d6e7f"

CFG = InstanceConfig(
    ch_host="localhost", ch_http_port=8123, ch_user="u", ch_password="p", ch_database="d",
    mariadb_host="localhost", mariadb_port=3306, mariadb_user="u", mariadb_password="p",
    mariadb_database="d", source="test",
)


def test_cold_mode_without_a_key(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv(WARM_KEY_ENV, raising=False)
    assert warm_tenant(None, CANONICAL) is None


def test_the_option_wins_over_the_environment(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv(WARM_KEY_ENV, "from-env")
    assert warm_tenant("from-option", CANONICAL) == WarmTenant("from-option", CANONICAL)


def test_a_key_equal_to_the_canonical_tenant_is_refused() -> None:
    with pytest.raises(TransitionError, match="nothing to upgrade"):
        warm_tenant(CANONICAL, CANONICAL)


def test_a_missing_script_is_named(tmp_path: Path) -> None:
    with pytest.raises(TransitionError, match=r"migrate-tenant-identity\.sh"):
        apply_transition(CFG, WarmTenant("example-legacy", CANONICAL), repo_root=tmp_path)


def test_the_script_receives_both_tenants(tmp_path: Path) -> None:
    script = tmp_path / "src/ingestion/scripts/migrate-tenant-identity.sh"
    script.parent.mkdir(parents=True)
    out = tmp_path / "argv"
    script.write_text(f'printf "%s\\n" "$@" "$CLICKHOUSE_URL" > {out}\n')
    apply_transition(CFG, WarmTenant("example-legacy", CANONICAL), repo_root=tmp_path)
    assert out.read_text().splitlines() == [
        "--canonical-tenant", CANONICAL,
        "--legacy-tenant-key", "example-legacy",
        "http://localhost:8123",
    ]


def test_a_failing_script_fails_the_spec(tmp_path: Path) -> None:
    script = tmp_path / "src/ingestion/scripts/migrate-tenant-identity.sh"
    script.parent.mkdir(parents=True)
    script.write_text("echo refused >&2; exit 3\n")
    with pytest.raises(TransitionError, match="exited 3"):
        apply_transition(CFG, WarmTenant("example-legacy", CANONICAL), repo_root=tmp_path)
