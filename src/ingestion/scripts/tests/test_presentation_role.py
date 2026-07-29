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
        -v "$PWD/src/ingestion/scripts/bootstrap-db/clickhouse-access-management.xml":\\
/etc/clickhouse-server/users.d/zz-access-management.xml:ro \\
        clickhouse/clickhouse-server:25.7.5
    PRESENTATION_ROLE_TEST_CH_URL=http://localhost:38210 \\
    PRESENTATION_ROLE_TEST_CH_USER=insight \\
    PRESENTATION_ROLE_TEST_CH_PASSWORD=insight \\
        .venv/bin/python -m pytest tests/test_presentation_role.py -q
"""

from __future__ import annotations

import os
import urllib.error
import urllib.request
from pathlib import Path

import pytest

CH_URL = os.environ.get("PRESENTATION_ROLE_TEST_CH_URL")
CH_USER = os.environ.get("PRESENTATION_ROLE_TEST_CH_USER", "default")
CH_PASSWORD = os.environ.get("PRESENTATION_ROLE_TEST_CH_PASSWORD", "")

pytestmark = pytest.mark.skipif(
    not CH_URL, reason="set PRESENTATION_ROLE_TEST_CH_URL (admin with access_management) to run"
)

ROLE_SQL = Path(__file__).resolve().parent.parent / "bootstrap-db" / "presentation-role.sql"

PROBE_USER = "pres_role_probe"
PROBE_PASSWORD = "probe"


def _query(sql: str, *, user: str, password: str) -> tuple[bool, str]:
    """POST one statement over the HTTP interface. Returns (ok, body)."""
    req = urllib.request.Request(
        CH_URL.rstrip("/") + "/", data=sql.encode(), headers={"X-ClickHouse-User": user, "X-ClickHouse-Key": password}
    )
    try:
        with urllib.request.urlopen(req) as resp:  # noqa: S310 (trusted local URL)
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
    for db in ("silver", "person", "identity", "insight", "presentation"):
        assert _admin(f"CREATE DATABASE IF NOT EXISTS {db}")[0]
    assert _admin("CREATE TABLE IF NOT EXISTS silver.probe (x UInt8) ENGINE=MergeTree ORDER BY x")[0]

    _apply(ROLE_SQL)

    assert _admin(f"DROP USER IF EXISTS {PROBE_USER}")[0]
    ok, resp = _admin(f"CREATE USER {PROBE_USER} IDENTIFIED BY '{PROBE_PASSWORD}' DEFAULT ROLE presentation_ro")
    assert ok, resp
    assert _admin(f"GRANT presentation_ro TO {PROBE_USER}")[0]
    try:
        yield _probe
    finally:
        _admin("DROP TABLE IF EXISTS presentation.scratch")
        _admin(f"DROP USER IF EXISTS {PROBE_USER}")


def test_contract_is_read_only(probe) -> None:
    """SELECT on the contract is allowed; every write/DDL is denied."""
    assert probe("SELECT count() FROM silver.probe")[0], "contract SELECT must be allowed"
    for sql in (
        "INSERT INTO silver.probe VALUES (1)",
        "DROP TABLE silver.probe",
        "ALTER TABLE silver.probe ADD COLUMN y UInt8",
        "TRUNCATE TABLE silver.probe",
    ):
        ok, resp = probe(sql)
        assert not ok, f"contract must reject: {sql!r}"
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
