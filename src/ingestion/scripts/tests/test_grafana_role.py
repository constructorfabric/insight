"""Opt-in integration test: the `grafana_ro` role grant matrix (#2888).

Pins the SELECT-only guarantee against a real ClickHouse: run
bootstrap-db/provision-grafana-access.sh, and assert the `grafana` user it
creates can read every granted database but write nowhere — unlike
presentation_ro there is no writable namespace at all.

Reuses the presentation-role test server contract (same env vars), so one
throwaway ClickHouse runs both suites — see test_presentation_role.py for the
docker one-liner. Skipped unless a server is offered.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Callable, Iterator
from pathlib import Path

import pytest

QueryFn = Callable[[str], tuple[bool, str]]

CH_URL = os.environ.get("PRESENTATION_ROLE_TEST_CH_URL")
CH_USER = os.environ.get("PRESENTATION_ROLE_TEST_CH_USER", "default")
CH_PASSWORD = os.environ.get("PRESENTATION_ROLE_TEST_CH_PASSWORD", "")

pytestmark = pytest.mark.skipif(
    not CH_URL, reason="set PRESENTATION_ROLE_TEST_CH_URL (admin with access_management) to run"
)

BOOTSTRAP_DIR = Path(__file__).resolve().parent.parent / "bootstrap-db"
PROVISION_SCRIPT = BOOTSTRAP_DIR / "provision-grafana-access.sh"

# Every database grafana_ro grants SELECT on — and nothing but SELECT.
GRANTED_DBS = ("silver", "identity", "insight", "presentation", "product_usage", "ingestion_history")

GRAFANA_USER = "grafana"
# Alphanumeric to satisfy the script's quote/`;` guard.
GRAFANA_PASSWORD = "grafanaTest2888"


def _query(sql: str, *, user: str, password: str) -> tuple[bool, str]:
    """POST one statement over the HTTP interface. Returns (ok, body)."""
    url = CH_URL.rstrip("/") + "/"
    # Pin the scheme: urllib honours file:// etc. CH_URL is an operator-supplied
    # test endpoint, but reject anything but http(s) so a stray value can't read
    # local files.
    if urllib.parse.urlparse(url).scheme not in ("http", "https"):
        raise ValueError(f"PRESENTATION_ROLE_TEST_CH_URL must be http(s), got {CH_URL!r}")
    req = urllib.request.Request(
        url, data=sql.encode(), headers={"X-ClickHouse-User": user, "X-ClickHouse-Key": password}
    )
    try:
        # nosemgrep: python.lang.security.audit.dynamic-urllib-use-detected.dynamic-urllib-use-detected
        with urllib.request.urlopen(req) as resp:  # noqa: S310 (scheme pinned to http(s) above)
            return True, resp.read().decode()
    except urllib.error.HTTPError as e:
        return False, e.read().decode()


def _admin(sql: str) -> tuple[bool, str]:
    return _query(sql, user=CH_USER, password=CH_PASSWORD)


def _run_provisioning() -> None:
    """Run provision-grafana-access.sh against the live server. The script
    talks to CH over curl (lib/ch-exec.sh) using these env names."""
    env = {
        **os.environ,
        "CLICKHOUSE_URL": CH_URL,
        "CLICKHOUSE_USER": CH_USER,
        "CLICKHOUSE_PASSWORD": CH_PASSWORD,
        "CLICKHOUSE_GRAFANA_PASSWORD": GRAFANA_PASSWORD,
    }
    result = subprocess.run(["bash", str(PROVISION_SCRIPT)], env=env, capture_output=True, text=True, timeout=60)
    assert result.returncode == 0, f"provisioning failed: {result.stdout}\n{result.stderr}"
    assert "grafana user ready" in result.stdout, result.stdout


@pytest.fixture(scope="module")
def grafana_user() -> Iterator[QueryFn]:
    """Run provision-grafana-access.sh against the live server and yield a
    query fn bound to the `grafana` user it creates. Torn down afterwards."""
    if not (shutil.which("bash") and shutil.which("curl")):
        pytest.skip("provision-grafana-access.sh needs bash + curl")

    for db in GRANTED_DBS:
        assert _admin(f"CREATE DATABASE IF NOT EXISTS {db}")[0]
        assert _admin(f"CREATE TABLE IF NOT EXISTS {db}.probe (x UInt8) ENGINE=MergeTree ORDER BY x")[0]

    _run_provisioning()

    def _query_as_grafana(sql: str) -> tuple[bool, str]:
        return _query(sql, user=GRAFANA_USER, password=GRAFANA_PASSWORD)

    try:
        yield _query_as_grafana
    finally:
        for db in GRANTED_DBS:
            _admin(f"DROP TABLE IF EXISTS {db}.probe")
        _admin(f"DROP USER IF EXISTS {GRAFANA_USER}")


@pytest.mark.parametrize("db", GRANTED_DBS)
def test_grafana_user_is_select_only_everywhere(grafana_user: QueryFn, db: str) -> None:
    """The `grafana` user reads every granted DB but cannot write or DDL anywhere."""
    assert grafana_user(f"SELECT count() FROM {db}.probe")[0], f"{db} SELECT must be allowed"
    for sql in (
        f"INSERT INTO {db}.probe VALUES (1)",
        f"CREATE TABLE {db}.made_up (x UInt8) ENGINE=MergeTree ORDER BY x",
        f"DROP TABLE {db}.probe",
        f"ALTER TABLE {db}.probe ADD COLUMN y UInt8",
        f"TRUNCATE TABLE {db}.probe",
    ):
        ok, resp = grafana_user(sql)
        assert not ok, f"{db} must reject: {sql!r}"
        assert "ACCESS_DENIED" in resp, resp


def test_provisioning_converges_a_warm_user(grafana_user: QueryFn) -> None:
    """Re-running provisioning strips privileges the user gained out-of-band.

    `ALTER USER ... DEFAULT ROLE` would only change role activation — a direct
    grant or a stray role would survive it. The script converges in place
    (REVOKE ALL for direct grants, an explicit revoke per extra role), so both
    must be gone after the next deploy hook while SELECT keeps working.
    """
    assert _admin(f"GRANT INSERT ON silver.probe TO {GRAFANA_USER}")[0]
    assert _admin("CREATE ROLE IF NOT EXISTS stray_ro")[0]
    assert _admin("GRANT CREATE ON presentation.* TO stray_ro")[0]
    assert _admin(f"GRANT stray_ro TO {GRAFANA_USER}")[0]
    assert grafana_user("INSERT INTO silver.probe VALUES (1)")[0], "out-of-band INSERT should work pre-converge"

    try:
        _run_provisioning()

        for sql in ("INSERT INTO silver.probe VALUES (1)", "SET ROLE stray_ro"):
            ok, resp = grafana_user(sql)
            assert not ok, f"re-provisioning must strip the out-of-band access: {sql!r}"
        assert grafana_user("SELECT count() FROM silver.probe")[0], "SELECT must survive re-provisioning"
    finally:
        _admin("DROP ROLE IF EXISTS stray_ro")
