"""
MariaDB identity seed: persons, org_chart, account_person_map.

All UUIDs are stored as BINARY(16) in RFC 4122 big-endian, matching
the .NET identity service's `Guid.ToByteArray(bigEndian: true)`
convention.
"""

from __future__ import annotations

import logging
import os
import uuid as uuid_mod
from collections.abc import Iterable, Iterator
from contextlib import contextmanager

import pymysql

from profiles import (
    AUTHOR_PERSON_UUID,
    DEV_SEED_SOURCE_ID,
    DEV_SEED_SOURCE_TYPE,
    ORG_CHART_SOURCE_TYPE,
    TEAM_PROFILES,
    Person,
    build_roster,
    get_dev_user_email,
)

LOG = logging.getLogger("seed.identity")


def _bin(u: str) -> bytes:
    """UUID string → 16 raw bytes, RFC 4122 big-endian."""
    return uuid_mod.UUID(u).bytes


@contextmanager
def _connect() -> Iterator[pymysql.connections.Connection]:
    host = os.environ.get("MARIADB_HOST", "mariadb")
    port = int(os.environ.get("MARIADB_PORT", "3306"))
    user = os.environ.get("MARIADB_USER", "insight")
    pwd = os.environ.get("MARIADB_PASSWORD", "insight-local")
    db = os.environ.get("MARIADB_DB", "identity")
    conn = pymysql.connect(
        host=host,
        port=port,
        user=user,
        password=pwd,
        database=db,
        autocommit=False,
        cursorclass=pymysql.cursors.Cursor,
    )
    try:
        yield conn
        conn.commit()
    except Exception:
        conn.rollback()
        raise
    finally:
        conn.close()


def seed_persons(
    cur: pymysql.cursors.Cursor,
    tenant_uuid: str,
    roster: Iterable[Person],
) -> int:
    """Insert one observation row per person (value_type='email').

    The unique key on `persons` is
    (tenant, person, source_type, source_id, value_type, value_hash).
    INSERT IGNORE absorbs re-runs cleanly.
    """
    sql = """
        INSERT IGNORE INTO persons (
            value_type, insight_source_type, insight_source_id,
            insight_tenant_id, value_id,
            person_id, author_person_id, reason
        ) VALUES (
            'email', %s, %s, %s, %s, %s, %s, %s
        )
    """
    rows = [
        (
            DEV_SEED_SOURCE_TYPE,
            _bin(DEV_SEED_SOURCE_ID),
            _bin(tenant_uuid),
            p.email,
            _bin(p.uuid),
            _bin(AUTHOR_PERSON_UUID),
            "seed.py demo roster",
        )
        for p in roster
    ]
    cur.executemany(sql, rows)
    return cur.rowcount


def seed_person_names(
    cur: pymysql.cursors.Cursor,
    tenant_uuid: str,
    roster: Iterable[Person],
) -> int:
    """Insert display_name / first_name / last_name observations per person.

    The identity service routes these value_types into `value_full_text`
    (not `value_id`); see seed-persons-from-identity-input.py's
    VALUE_TYPES_FOR_VALUE_FULL_TEXT. Without them the persons API returns
    empty names and the UI falls back to email. INSERT IGNORE absorbs
    re-runs (unique key includes value_type).
    """
    sql = """
        INSERT IGNORE INTO persons (
            value_type, insight_source_type, insight_source_id,
            insight_tenant_id, value_full_text,
            person_id, author_person_id, reason
        ) VALUES (
            %s, %s, %s, %s, %s, %s, %s, %s
        )
    """
    rows: list[tuple[object, ...]] = []
    for p in roster:
        for value_type, value in (
            ("display_name", p.display_name),
            ("first_name", p.first_name),
            ("last_name", p.last_name),
        ):
            if not value:
                continue
            rows.append((
                value_type,
                DEV_SEED_SOURCE_TYPE,
                _bin(DEV_SEED_SOURCE_ID),
                _bin(tenant_uuid),
                value,
                _bin(p.uuid),
                _bin(AUTHOR_PERSON_UUID),
                "seed.py demo names",
            ))
    cur.executemany(sql, rows)
    return cur.rowcount


def seed_org_chart(
    cur: pymysql.cursors.Cursor,
    tenant_uuid: str,
    roster: Iterable[Person],
) -> int:
    """One open-ended edge per non-CEO person."""
    sql = """
        INSERT IGNORE INTO org_chart (
            insight_tenant_id, insight_source_type, insight_source_id,
            child_person_id, parent_person_id,
            author_person_id, reason, valid_from
        ) VALUES (
            %s, %s, %s, %s, %s, %s, %s, '2000-01-01 00:00:00'
        )
    """
    rows = [
        (
            _bin(tenant_uuid),
            ORG_CHART_SOURCE_TYPE,
            _bin(DEV_SEED_SOURCE_ID),
            _bin(p.uuid),
            _bin(p.parent_uuid),
            _bin(AUTHOR_PERSON_UUID),
            "seed.py demo org-chart",
        )
        for p in roster
        if p.parent_uuid
    ]
    cur.executemany(sql, rows)
    return cur.rowcount


def seed_account_person_map(
    cur: pymysql.cursors.Cursor,
    tenant_uuid: str,
    roster: Iterable[Person],
) -> int:
    """Per (person, source_type) row where the team has non-zero weight."""
    sql = """
        INSERT IGNORE INTO account_person_map (
            insight_tenant_id, insight_source_type, insight_source_id,
            source_account_id, person_id,
            author_person_id, reason, valid_from
        ) VALUES (
            %s, %s, %s, %s, %s, %s, %s, '2000-01-01 00:00:00'
        )
    """
    rows: list[tuple[object, ...]] = []
    for p in roster:
        # CEO doesn't get source accounts — observed via roll-ups only.
        if not p.team:
            continue
        profile = TEAM_PROFILES.get(p.team)
        if not profile:
            continue
        for source_type, weight in profile.weights.items():
            if weight <= 0:
                continue
            rows.append((
                _bin(tenant_uuid),
                source_type,
                _bin(DEV_SEED_SOURCE_ID),
                p.email,
                _bin(p.uuid),
                _bin(AUTHOR_PERSON_UUID),
                "seed.py account-person map",
            ))
    cur.executemany(sql, rows)
    return cur.rowcount


def run() -> None:
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s %(levelname)s %(name)s %(message)s",
    )

    tenant = os.environ.get(
        "TENANT_DEFAULT_ID", "00000000-df51-5b42-9538-d2b56b7ee953"
    )
    dev_email = get_dev_user_email()
    roster = build_roster(dev_email)
    LOG.info(
        "seeding %d persons under tenant %s (dev lead = %s)",
        len(roster), tenant, dev_email,
    )

    with _connect() as conn:
        cur = conn.cursor()
        n_persons = seed_persons(cur, tenant, roster)
        n_names = seed_person_names(cur, tenant, roster)
        n_org = seed_org_chart(cur, tenant, roster)
        n_acct = seed_account_person_map(cur, tenant, roster)

    LOG.info(
        "DONE: persons=%d (new), names=%d (new), org_chart=%d (new), "
        "account_person_map=%d (new)",
        n_persons, n_names, n_org, n_acct,
    )


if __name__ == "__main__":
    run()
