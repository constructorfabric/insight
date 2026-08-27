"""Opt-in integration test: the `presentation_ro` role grant matrix (#1963).

Pins the read-only-by-construction guarantee against a real ClickHouse: apply
bootstrap-db/presentation-role.sql, assign the role to a throwaway probe user,
and assert what that user can and cannot do. This is the adversarial-write half
of NFR `cpt-presentation-nfr-source-immutability` — the contract is read-only,
`presentation` is create/insert-only, and nothing can DROP/ALTER/TRUNCATE.

Skipped unless a server is offered, so CI and local `pytest` stay dependency-
free. The admin must have access_management (to CREATE ROLE / CREATE USER):

    docker run -d --rm --name ch -p 38210:8123 \\
        -e CLICKHOUSE_USER=insight -e CLICKHOUSE_PASSWORD=insight \\
        -e CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT=1 \\
        clickhouse/clickhouse-server:25.7.5
    PRESENTATION_ROLE_TEST_CH_URL=http://localhost:38210 \\
    PRESENTATION_ROLE_TEST_CH_USER=insight \\
    PRESENTATION_ROLE_TEST_CH_PASSWORD=insight \\
        .venv/bin/python -m pytest tests/test_presentation_role.py -q
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

CH_URL = os.environ.get("PRESENTATION_ROLE_TEST_CH_URL")
CH_USER = os.environ.get("PRESENTATION_ROLE_TEST_CH_USER", "default")
CH_PASSWORD = os.environ.get("PRESENTATION_ROLE_TEST_CH_PASSWORD", "")

pytestmark = pytest.mark.skipif(
    not CH_URL, reason="set PRESENTATION_ROLE_TEST_CH_URL (admin with access_management) to run"
)

BOOTSTRAP_DIR = Path(__file__).resolve().parent.parent / "bootstrap-db"
ROLE_SQL = BOOTSTRAP_DIR / "presentation-role.sql"
PROVISION_SCRIPT = BOOTSTRAP_DIR / "provision-presentation-access.sh"

# The read-only contract databases the role grants SELECT on.
CONTRACT_DBS = ("silver", "person", "identity", "insight")

# Append-only: SELECT + INSERT, but no CREATE — its DDL comes from migrations.
USAGE_DB = "product_usage"

# Read-only: SELECT and nothing else. Its writer is the reconcile loop, which
# authenticates as the ingestion admin, so the query path never needs INSERT.
HISTORY_DB = "ingestion_history"

PROBE_USER = "pres_role_probe"
PROBE_PASSWORD = "probe"

# The persistent grant-less user provision-presentation-access.sh creates and
# that analytics connects as (#1964). Alphanumeric to satisfy the script's
# quote/`;` guard.
PRES_USER = "presentation"
PRES_PASSWORD = "presTest1964"


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


def _probe(sql: str) -> tuple[bool, str]:
    return _query(sql, user=PROBE_USER, password=PROBE_PASSWORD)


def _apply(path: Path) -> None:
    """Fan the SQL file out statement-by-statement, mirroring lib/ch-exec.sh
    run_ch: drop full-line `--` comments, split on `;`."""
    body = "\n".join(line for line in path.read_text().splitlines() if not line.lstrip().startswith("--"))
    for stmt in body.split(";"):
        if stmt.strip():
            ok, resp = _admin(stmt)
            assert ok, f"admin stmt failed: {stmt.strip()!r} -> {resp}"


@pytest.fixture(scope="module")
def probe():
    """Provision the contract + presentation objects and a probe user carrying
    only the presentation_ro role. Torn down afterwards."""
    for db in (*CONTRACT_DBS, "presentation", USAGE_DB, HISTORY_DB):
        assert _admin(f"CREATE DATABASE IF NOT EXISTS {db}")[0]
    for db in CONTRACT_DBS:
        assert _admin(f"CREATE TABLE IF NOT EXISTS {db}.probe (x UInt8) ENGINE=MergeTree ORDER BY x")[0]

    _apply(ROLE_SQL)

    # ClickHouse only accepts a default role once it is granted, so grant first.
    assert _admin(f"DROP USER IF EXISTS {PROBE_USER}")[0]
    ok, resp = _admin(f"CREATE USER {PROBE_USER} IDENTIFIED BY '{PROBE_PASSWORD}'")
    assert ok, resp
    assert _admin(f"GRANT presentation_ro TO {PROBE_USER}")[0]
    assert _admin(f"ALTER USER {PROBE_USER} DEFAULT ROLE presentation_ro")[0]
    try:
        yield _probe
    finally:
        _admin("DROP TABLE IF EXISTS presentation.scratch")
        _admin(f"DROP TABLE IF EXISTS {USAGE_DB}.probe")
        _admin(f"DROP TABLE IF EXISTS {HISTORY_DB}.probe")
        for db in CONTRACT_DBS:
            _admin(f"DROP TABLE IF EXISTS {db}.probe")
        _admin(f"DROP USER IF EXISTS {PROBE_USER}")


@pytest.mark.parametrize("db", CONTRACT_DBS)
def test_contract_is_read_only(probe, db: str) -> None:
    """SELECT on every contract database is allowed; every write/DDL is denied."""
    assert probe(f"SELECT count() FROM {db}.probe")[0], f"{db} SELECT must be allowed"
    for sql in (
        f"INSERT INTO {db}.probe VALUES (1)",
        f"DROP TABLE {db}.probe",
        f"ALTER TABLE {db}.probe ADD COLUMN y UInt8",
        f"TRUNCATE TABLE {db}.probe",
    ):
        ok, resp = probe(sql)
        assert not ok, f"{db} must reject: {sql!r}"
        assert "ACCESS_DENIED" in resp, resp


def test_presentation_is_create_insert_only(probe) -> None:
    """CREATE/INSERT/SELECT allowed in presentation; DROP/ALTER/TRUNCATE denied."""
    assert probe("CREATE TABLE IF NOT EXISTS presentation.scratch (x UInt8) ENGINE=MergeTree ORDER BY x")[0], (
        "presentation CREATE must be allowed"
    )
    assert probe("INSERT INTO presentation.scratch VALUES (7)")[0], "presentation INSERT must be allowed"
    assert probe("SELECT sum(x) FROM presentation.scratch")[0], "presentation SELECT must be allowed"
    for sql in (
        "DROP TABLE presentation.scratch",
        "TRUNCATE TABLE presentation.scratch",
        "ALTER TABLE presentation.scratch ADD COLUMN y UInt8",
    ):
        ok, resp = probe(sql)
        assert not ok, f"presentation must reject destructive DDL: {sql!r}"
        assert "ACCESS_DENIED" in resp, resp


def test_usage_is_append_only(probe) -> None:
    """INSERT/SELECT allowed in product_usage; CREATE denied along with the rest.

    The distinction from `presentation`: this database's DDL is owned by
    `scripts/migrations/`, so the service that writes adoption events cannot
    define a table anywhere. A regression that granted CREATE back would let the
    schema drift away from the migration silently.
    """
    assert _admin(
        f"CREATE TABLE IF NOT EXISTS {USAGE_DB}.probe (x UInt8) ENGINE=MergeTree ORDER BY x"
    )[0]
    assert probe(f"INSERT INTO {USAGE_DB}.probe VALUES (7)")[0], "usage INSERT must be allowed"
    assert probe(f"SELECT sum(x) FROM {USAGE_DB}.probe")[0], "usage SELECT must be allowed"
    for sql in (
        f"CREATE TABLE {USAGE_DB}.made_up (x UInt8) ENGINE=MergeTree ORDER BY x",
        f"DROP TABLE {USAGE_DB}.probe",
        f"TRUNCATE TABLE {USAGE_DB}.probe",
        f"ALTER TABLE {USAGE_DB}.probe ADD COLUMN y UInt8",
    ):
        ok, resp = probe(sql)
        assert not ok, f"{USAGE_DB} must reject: {sql!r}"
        assert "ACCESS_DENIED" in resp, resp


def test_sync_history_is_read_only(probe) -> None:
    """SELECT allowed in ingestion_history; INSERT denied along with the rest.

    The distinction from `product_usage`: adoption events are written by the
    same service that serves them, so that database needs INSERT. Sync history
    is written by the reconcile loop under different credentials entirely, so a
    grant letting the query path write here would be a grant nothing uses — and
    the surface reporting on ingestion could edit its own evidence.
    """
    assert _admin(
        f"CREATE TABLE IF NOT EXISTS {HISTORY_DB}.probe (x UInt8) ENGINE=MergeTree ORDER BY x"
    )[0]
    assert probe(f"SELECT sum(x) FROM {HISTORY_DB}.probe")[0], "history SELECT must be allowed"
    for sql in (
        f"INSERT INTO {HISTORY_DB}.probe VALUES (7)",
        f"CREATE TABLE {HISTORY_DB}.made_up (x UInt8) ENGINE=MergeTree ORDER BY x",
        f"DROP TABLE {HISTORY_DB}.probe",
        f"TRUNCATE TABLE {HISTORY_DB}.probe",
        f"ALTER TABLE {HISTORY_DB}.probe ADD COLUMN y UInt8",
    ):
        ok, resp = probe(sql)
        assert not ok, f"{HISTORY_DB} must reject: {sql!r}"
        assert "ACCESS_DENIED" in resp, resp


# ── #1964: the persistent grant-less `presentation` user, provisioned by the
#    real provision-presentation-access.sh (not a throwaway probe) ──


@pytest.fixture(scope="module")
def presentation_user():
    """Run provision-presentation-access.sh against the live server and yield a
    query fn bound to the `presentation` user it creates. Proves the user
    analytics actually connects as (#1964) carries exactly the role's grant
    matrix — a grant-less user whose only privileges come via presentation_ro."""
    if not (shutil.which("bash") and shutil.which("curl")):
        pytest.skip("provision-presentation-access.sh needs bash + curl")

    for db in (*CONTRACT_DBS, "presentation", USAGE_DB, HISTORY_DB):
        assert _admin(f"CREATE DATABASE IF NOT EXISTS {db}")[0]
    for db in CONTRACT_DBS:
        assert _admin(f"CREATE TABLE IF NOT EXISTS {db}.probe (x UInt8) ENGINE=MergeTree ORDER BY x")[0]

    # The script talks to CH over curl (lib/ch-exec.sh) using these env names.
    env = {
        **os.environ,
        "CLICKHOUSE_URL": CH_URL,
        "CLICKHOUSE_USER": CH_USER,
        "CLICKHOUSE_PASSWORD": CH_PASSWORD,
        "CLICKHOUSE_PRESENTATION_PASSWORD": PRES_PASSWORD,
    }
    result = subprocess.run(["bash", str(PROVISION_SCRIPT)], env=env, capture_output=True, text=True, timeout=60)
    assert result.returncode == 0, f"provisioning failed: {result.stdout}\n{result.stderr}"
    assert "presentation user ready" in result.stdout, result.stdout

    def _query_as_pres(sql: str) -> tuple[bool, str]:
        return _query(sql, user=PRES_USER, password=PRES_PASSWORD)

    try:
        yield _query_as_pres
    finally:
        _admin("DROP TABLE IF EXISTS presentation.scratch_1964")
        for db in CONTRACT_DBS:
            _admin(f"DROP TABLE IF EXISTS {db}.probe")
        _admin(f"DROP USER IF EXISTS {PRES_USER}")


@pytest.mark.parametrize("db", CONTRACT_DBS)
def test_provisioned_user_contract_is_read_only(presentation_user, db: str) -> None:
    """The `presentation` user reads every contract DB but cannot write/alter it."""
    assert presentation_user(f"SELECT count() FROM {db}.probe")[0], f"{db} SELECT must be allowed"
    for sql in (
        f"INSERT INTO {db}.probe VALUES (1)",
        f"DROP TABLE {db}.probe",
        f"ALTER TABLE {db}.probe ADD COLUMN y UInt8",
        f"TRUNCATE TABLE {db}.probe",
    ):
        ok, resp = presentation_user(sql)
        assert not ok, f"{db} must reject: {sql!r}"
        assert "ACCESS_DENIED" in resp, resp


def test_provisioned_user_presentation_is_create_insert_only(presentation_user) -> None:
    """The `presentation` user can CREATE/INSERT/SELECT in `presentation` (proving
    the DB exists and is writable) but cannot DROP/ALTER/TRUNCATE there."""
    assert presentation_user(
        "CREATE TABLE IF NOT EXISTS presentation.scratch_1964 (x UInt8) ENGINE=MergeTree ORDER BY x"
    )[0], "presentation CREATE must be allowed"
    assert presentation_user("INSERT INTO presentation.scratch_1964 VALUES (7)")[0], "INSERT must be allowed"
    assert presentation_user("SELECT sum(x) FROM presentation.scratch_1964")[0], "SELECT must be allowed"
    for sql in (
        "DROP TABLE presentation.scratch_1964",
        "TRUNCATE TABLE presentation.scratch_1964",
        "ALTER TABLE presentation.scratch_1964 ADD COLUMN y UInt8",
    ):
        ok, resp = presentation_user(sql)
        assert not ok, f"presentation must reject destructive DDL: {sql!r}"
        assert "ACCESS_DENIED" in resp, resp
