"""Opt-in integration test: the `grafana_ro` role grant matrix.

Pins the read-only guarantee for Grafana's ClickHouse datasource against a real
ClickHouse: run provision-clickhouse-user.sh in direct mode, then assert the
user it creates can read every database — including a `bronze_*` created after
the grant, which is the whole reason the grant is a wildcard — and cannot
write, alter or drop anything anywhere.

Skipped unless a server is offered, so CI and local `pytest` stay dependency-
free. The admin must have access_management (to CREATE ROLE / CREATE USER):

    docker run -d --rm --name ch -p 38211:8123 \\
        -e CLICKHOUSE_USER=insight -e CLICKHOUSE_PASSWORD=insight \\
        -e CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT=1 \\
        clickhouse/clickhouse-server:25.7.5
    GRAFANA_ROLE_TEST_CH_URL=http://localhost:38211 \\
    GRAFANA_ROLE_TEST_CH_USER=insight \\
    GRAFANA_ROLE_TEST_CH_PASSWORD=insight \\
        python -m pytest deploy/gitops/system/grafana/tests -q
"""

from __future__ import annotations

import os
import shutil
import subprocess
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

import pytest

CH_URL = os.environ.get("GRAFANA_ROLE_TEST_CH_URL")
CH_USER = os.environ.get("GRAFANA_ROLE_TEST_CH_USER", "default")
CH_PASSWORD = os.environ.get("GRAFANA_ROLE_TEST_CH_PASSWORD", "")

pytestmark = pytest.mark.skipif(not CH_URL, reason="set GRAFANA_ROLE_TEST_CH_URL (admin with access_management) to run")

PROVISION_SCRIPT = Path(__file__).resolve().parent.parent / "provision-clickhouse-user.sh"

GRAFANA_USER = "grafana_ro"
GRAFANA_PASSWORD = "grafanaRoTest1"

# `silver` and `insight` stand in for the fixed databases; the bronze ones
# exercise the wildcard grant — LATE_DB is created after the role is granted,
# which an enumerated grant list would miss.
SEEDED_DBS = ("silver", "insight", "bronze_probe")
LATE_DB = "bronze_onboarded_later"


def _query(sql: str, *, user: str, password: str) -> tuple[bool, str]:
    """POST one statement over the HTTP interface. Returns (ok, body)."""
    url = CH_URL.rstrip("/") + "/"
    # Pin the scheme: urllib honours file:// etc. CH_URL is an operator-supplied
    # test endpoint, but reject anything but http(s) so a stray value can't read
    # local files.
    if urllib.parse.urlparse(url).scheme not in ("http", "https"):
        raise ValueError(f"GRAFANA_ROLE_TEST_CH_URL must be http(s), got {CH_URL!r}")
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


def _seed(db: str) -> None:
    assert _admin(f"CREATE DATABASE IF NOT EXISTS {db}")[0]
    assert _admin(f"CREATE TABLE IF NOT EXISTS {db}.probe (x UInt8) ENGINE=MergeTree ORDER BY x")[0]
    assert _admin(f"INSERT INTO {db}.probe VALUES (1)")[0]


def _provision(*, with_password: bool) -> subprocess.CompletedProcess[str]:
    """Run the script in direct mode — CLICKHOUSE_URL set, so it never touches kubectl."""
    env = {**os.environ, "CLICKHOUSE_URL": CH_URL, "CLICKHOUSE_USER": CH_USER, "CLICKHOUSE_PASSWORD": CH_PASSWORD}
    if with_password:
        env["CLICKHOUSE_GRAFANA_PASSWORD"] = GRAFANA_PASSWORD
    else:
        env.pop("CLICKHOUSE_GRAFANA_PASSWORD", None)
    return subprocess.run(["bash", str(PROVISION_SCRIPT)], env=env, capture_output=True, text=True, timeout=60)


@pytest.fixture(scope="module")
def grafana_user():
    if not (shutil.which("bash") and shutil.which("curl")):
        pytest.skip("provision-clickhouse-user.sh needs bash + curl")

    for db in SEEDED_DBS:
        _seed(db)

    result = _provision(with_password=True)
    assert result.returncode == 0, f"provisioning failed: {result.stdout}\n{result.stderr}"
    assert "grafana_ro user ready" in result.stdout, result.stdout

    def _query_as_grafana(sql: str) -> tuple[bool, str]:
        return _query(sql, user=GRAFANA_USER, password=GRAFANA_PASSWORD)

    try:
        yield _query_as_grafana
    finally:
        for db in (*SEEDED_DBS, LATE_DB):
            _admin(f"DROP DATABASE IF EXISTS {db}")
        _admin(f"DROP USER IF EXISTS {GRAFANA_USER}")


@pytest.mark.parametrize("db", SEEDED_DBS)
def test_every_database_is_readable(grafana_user, db: str) -> None:
    assert grafana_user(f"SELECT count() FROM {db}.probe")[0], f"{db} SELECT must be allowed"


def test_database_created_after_the_grant_is_readable(grafana_user) -> None:
    """A connector onboarded after provisioning must not need a re-grant — the
    reason grafana_ro holds SELECT ON *.* rather than an enumerated list."""
    _seed(LATE_DB)
    assert grafana_user(f"SELECT count() FROM {LATE_DB}.probe")[0]


@pytest.mark.parametrize("db", SEEDED_DBS)
def test_writes_are_refused_everywhere(grafana_user, db: str) -> None:
    for sql in (
        f"INSERT INTO {db}.probe VALUES (2)",
        f"DROP TABLE {db}.probe",
        f"ALTER TABLE {db}.probe ADD COLUMN y UInt8",
        f"TRUNCATE TABLE {db}.probe",
        f"CREATE TABLE {db}.evil (x UInt8) ENGINE=MergeTree ORDER BY x",
    ):
        ok, resp = grafana_user(sql)
        assert not ok, f"{db} must reject: {sql!r}"
        assert "ACCESS_DENIED" in resp or "READONLY" in resp, resp


def test_cannot_escalate_its_own_access(grafana_user) -> None:
    """readonly=2 plus a SELECT-only role: no access management, and no lifting
    the readonly setting for the session."""
    for sql in ("CREATE USER escalated IDENTIFIED BY 'x'", "GRANT INSERT ON *.* TO grafana_ro", "SET readonly = 0"):
        ok, _ = grafana_user(sql)
        assert not ok, f"must reject: {sql!r}"


def test_rerun_converges_an_existing_user(grafana_user) -> None:
    """`make system-grafana` re-runs this on every deploy, against a cluster
    where the user already exists."""
    result = _provision(with_password=True)
    assert result.returncode == 0, f"re-run failed: {result.stdout}\n{result.stderr}"
    assert grafana_user("SELECT 1")[0], "user must still authenticate after a re-run"


def test_role_only_when_password_unset() -> None:
    """No grafana-clickhouse Secret yet → the role exists and the user does not,
    so installing Grafana before sealing the password degrades rather than fails."""
    assert _admin(f"DROP USER IF EXISTS {GRAFANA_USER}")[0]

    result = _provision(with_password=False)
    assert result.returncode == 0, result.stderr
    assert "role only" in result.stdout, result.stdout

    try:
        ok, users = _admin(f"SELECT count() FROM system.users WHERE name = '{GRAFANA_USER}'")
        assert ok and users.strip() == "0", f"user must not be created: {users!r}"
        ok, roles = _admin(f"SELECT count() FROM system.roles WHERE name = '{GRAFANA_USER}'")
        assert ok and roles.strip() == "1", f"role must exist: {roles!r}"
    finally:
        _admin(f"DROP ROLE IF EXISTS {GRAFANA_USER}")
