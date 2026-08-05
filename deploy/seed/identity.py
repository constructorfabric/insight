"""
MariaDB identity seed: persons, org_chart, person_roles, account_person_map.

All UUIDs are stored as BINARY(16) in RFC 4122 big-endian — the
convention the identity-resolution service's schema uses (inherited
from the retired .NET service's `Guid.ToByteArray(bigEndian: true)`).
"""

from __future__ import annotations

import logging
import os
import uuid as uuid_mod
from collections.abc import Iterable, Iterator
from contextlib import contextmanager

import pymysql

from profiles import (
    ADMIN_ROLE_NAME,
    AUTHOR_PERSON_UUID,
    DEV_SEED_SOURCE_ID,
    DEV_SEED_SOURCE_TYPE,
    ORG_CHART_SOURCE_TYPE,
    TEAM_PROFILES,
    TENANT_OTHER,
    Person,
    build_other_tenant_roster,
    build_roster,
    get_dev_user_email,
    get_idp_source_type,
    get_login_id_pairs,
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


def seed_login_ids(
    cur: pymysql.cursors.Cursor,
    tenant_uuid: str,
    roster: Iterable[Person],
) -> int:
    """Insert `value_type='id'` login-bootstrap observations under the login
    IdP's source_type — the rows the authenticator's login-bootstrap resolve
    (`GET /internal/persons/by-external-id?source_type=...&external_id=...`)
    looks up. Without them a fresh dev/demo/CI stack can authenticate against
    the IdP but never resolves to a person (403 at callback).

    WHICH persona(s) get a row depends on the active IdP fixture (see
    `profiles.get_login_id_pairs`): fakeidp only defines the dev lead; a
    Keycloak realm seeds the WHOLE roster, so every one of those 25 personas
    must get their own row here too, or logging in as anyone but the dev lead
    403s despite Keycloak having authenticated them correctly.

    Idempotent via an explicit existence check per pair, NOT `INSERT IGNORE`:
    since migration 004 (`004_persons_relax_constraints.sql`), `persons`'
    unique key is `(tenant, person, source_type, source_id, value_type,
    created_at)` — `created_at` replaced `value_hash` so the append-only
    observation log can record the same value re-observed at a different
    time. That means an `INSERT IGNORE` re-run with a fresh `created_at`
    never collides with the unique key and always inserts a new row. A plain
    existence check on the logical key (ignoring `created_at`) is what
    actually makes re-runs a no-op here.
    """
    source_type = get_idp_source_type()
    exists_sql = """
        SELECT 1 FROM persons
        WHERE insight_tenant_id = %s
          AND person_id = %s
          AND insight_source_type = %s
          AND insight_source_id = %s
          AND value_type = 'id'
          AND value_id = %s
        LIMIT 1
    """
    insert_sql = """
        INSERT INTO persons (
            value_type, insight_source_type, insight_source_id,
            insight_tenant_id, value_id,
            person_id, author_person_id, reason
        ) VALUES (
            'id', %s, %s, %s, %s, %s, %s, %s
        )
    """
    inserted = 0
    for person_uuid, external_id in get_login_id_pairs(list(roster)):
        cur.execute(exists_sql, (
            _bin(tenant_uuid),
            _bin(person_uuid),
            source_type,
            _bin(DEV_SEED_SOURCE_ID),
            external_id,
        ))
        if cur.fetchone() is not None:
            continue
        cur.execute(insert_sql, (
            source_type,
            _bin(DEV_SEED_SOURCE_ID),
            _bin(tenant_uuid),
            external_id,
            _bin(person_uuid),
            _bin(AUTHOR_PERSON_UUID),
            "seed.py login id",
        ))
        inserted += cur.rowcount
    return inserted


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
            rows.append(
                (
                    value_type,
                    DEV_SEED_SOURCE_TYPE,
                    _bin(DEV_SEED_SOURCE_ID),
                    _bin(tenant_uuid),
                    value,
                    _bin(p.uuid),
                    _bin(AUTHOR_PERSON_UUID),
                    "seed.py demo names",
                )
            )
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


def seed_person_roles(
    cur: pymysql.cursors.Cursor,
    tenant_uuid: str,
    roster: Iterable[Person],
) -> int:
    """Grant the `admin` role to every roster person whose role is `admin`.

    This — not a realm role — is what admits a caller to the admin-gated
    identity API. `require_admin` resolves the caller from the gateway JWT and
    looks for an active row here; it never reads `insight-admin` from the token.
    Without this step the whole admin surface answers 403 to everybody, the CEO
    included.

    The `roles` row itself is created by the identity-resolution migrations, so
    it is looked up by name rather than inserted: a missing one means the
    migrations have not run, and failing loudly beats seeding a dangling grant.
    """
    cur.execute("SELECT role_id FROM roles WHERE name = %s", (ADMIN_ROLE_NAME,))
    row = cur.fetchone()
    if row is None:
        raise RuntimeError(
            f"no {ADMIN_ROLE_NAME!r} row in identity.roles — the "
            "identity-resolution migrations have not been applied to this stand"
        )
    role_id = row[0]

    sql = """
        INSERT IGNORE INTO person_roles (
            person_role_id, insight_tenant_id, person_id, role_id,
            valid_from, valid_to, author_person_id, reason
        ) VALUES (
            %s, %s, %s, %s, '2000-01-01 00:00:00', NULL, %s, %s
        )
    """
    rows = [
        (
            # Derived from the person rather than random, so re-seeding is
            # idempotent: INSERT IGNORE can only absorb a repeat if the primary
            # key repeats too.
            _bin(str(uuid_mod.uuid5(uuid_mod.NAMESPACE_OID, f"person_role:{p.uuid}"))),
            _bin(tenant_uuid),
            _bin(p.uuid),
            role_id,
            _bin(AUTHOR_PERSON_UUID),
            "seed.py admin operator",
        )
        for p in roster
        if p.role == "admin"
    ]
    if not rows:
        return 0
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
            rows.append(
                (
                    _bin(tenant_uuid),
                    source_type,
                    _bin(DEV_SEED_SOURCE_ID),
                    p.email,
                    _bin(p.uuid),
                    _bin(AUTHOR_PERSON_UUID),
                    "seed.py account-person map",
                )
            )
    cur.executemany(sql, rows)
    return cur.rowcount


def run() -> None:
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s %(levelname)s %(name)s %(message)s",
    )

    tenant = os.environ.get("TENANT_DEFAULT_ID", "00000000-df51-5b42-9538-d2b56b7ee953")
    dev_email = get_dev_user_email()
    roster = build_roster(dev_email)
    LOG.info(
        "seeding %d persons under tenant %s (dev lead = %s)",
        len(roster),
        tenant,
        dev_email,
    )

    # The second tenant's population, seeded by the SAME functions rather than
    # by a variant of them: every row they write is already tenant-scoped, so
    # running them again under a different tenant is the whole difference. A
    # per-person tenant field would have had to thread through five writers and
    # every generator, to express one row.
    other_roster = build_other_tenant_roster()
    LOG.info(
        "seeding %d person(s) under tenant %s (cross-tenant refusal fixture)",
        len(other_roster),
        TENANT_OTHER,
    )

    with _connect() as conn:
        cur = conn.cursor()
        n_persons = seed_persons(cur, tenant, roster)
        n_login_id = seed_login_ids(cur, tenant, roster)
        n_names = seed_person_names(cur, tenant, roster)
        n_org = seed_org_chart(cur, tenant, roster)
        n_roles = seed_person_roles(cur, tenant, roster)
        n_acct = seed_account_person_map(cur, tenant, roster)

        # No org_chart and no person_roles for them: they are a caller, not a
        # subject. An edge would put them in somebody's subtree, and a role
        # would make the refusal ambiguous — is it the tenant or the grant?
        n_persons += seed_persons(cur, TENANT_OTHER, other_roster)
        n_login_id += seed_login_ids(cur, TENANT_OTHER, other_roster)
        n_names += seed_person_names(cur, TENANT_OTHER, other_roster)
        n_acct += seed_account_person_map(cur, TENANT_OTHER, other_roster)

    LOG.info(
        "DONE: persons=%d (new), login_id=%d (new), names=%d (new), "
        "org_chart=%d (new), person_roles=%d (new), account_person_map=%d (new)",
        n_persons,
        n_login_id,
        n_names,
        n_org,
        n_roles,
        n_acct,
    )


if __name__ == "__main__":
    run()
