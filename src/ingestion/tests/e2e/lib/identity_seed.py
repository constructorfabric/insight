"""Deterministic fixture dataset for the identity contract suite.

Inserted straight into the identity service's own MariaDB database AFTER the
service booted (its startup migrations create the schema). All UUIDs are fixed
so tests can assert exact ids; all rows carry reason='e2e-seed'.

The org tree (tenant = TEST_TENANT_ID, source = bamboohr/SOURCE_ID):

    alice (admin, root)
    ├── bob ── carol
    ├── dup1 ┐  same email dup@e2e.test → ambiguous-profile case
    └── dup2 ┘
    hidden (root of its own tree — OUTSIDE alice's subtree)
    eve    (OTHER_TENANT — cross-tenant isolation case)

Visibility semantics under test: a caller sees their own org_chart subtree
plus explicit grants (roles ≠ visibility — alice's admin role does NOT let
her see `hidden`). One explicit grant is seeded: bob → hidden.

Observation routing mirrors the seeder's ValueRouting: identifier types
(email, id, …) → value_id; human-readable attributes (display_name,
department, job_title, status, …) → value_full_text.
"""

from __future__ import annotations

import logging
import uuid

import pymysql

from lib.config import TEST_TENANT_ID, SessionConfig
from lib.identity import IDENTITY_DATABASE

LOG = logging.getLogger("e2e.identity")

# -- fixed identifiers ---------------------------------------------------

OTHER_TENANT = uuid.UUID("22222222-2222-2222-2222-222222222222")

SOURCE_TYPE = "bamboohr"
SOURCE_ID = uuid.UUID("33333333-3333-3333-3333-333333333333")

ADMIN_ROLE_ID = uuid.UUID("a4d11000-0000-4000-8000-000000000001")

ALICE = uuid.UUID("aaaaaaaa-0000-4000-8000-000000000001")
BOB = uuid.UUID("aaaaaaaa-0000-4000-8000-000000000002")
CAROL = uuid.UUID("aaaaaaaa-0000-4000-8000-000000000003")
DUP1 = uuid.UUID("aaaaaaaa-0000-4000-8000-000000000004")
DUP2 = uuid.UUID("aaaaaaaa-0000-4000-8000-000000000005")
HIDDEN = uuid.UUID("aaaaaaaa-0000-4000-8000-000000000006")
EVE = uuid.UUID("bbbbbbbb-0000-4000-8000-000000000001")

VISIBILITY_GRANT_BOB_HIDDEN = uuid.UUID("cccccccc-0000-4000-8000-000000000001")
ALICE_ADMIN_ASSIGNMENT = uuid.UUID("cccccccc-0000-4000-8000-000000000002")

# persons-seed runs under its OWN tenant so its tenant-scoped rebuild of
# account_person_map / org_chart never touches the fixture tree above.
# (The identity_inputs read is deliberately tenant-UNfiltered — hotfix #1550 —
# but every WRITE binds the caller's tenant.)
SEED_TENANT = uuid.UUID("44444444-4444-4444-4444-444444444444")
SEED_ADMIN = uuid.UUID("dddddddd-0000-4000-8000-000000000001")
SEED_ADMIN_ASSIGNMENT = uuid.UUID("cccccccc-0000-4000-8000-000000000003")

ALICE_EMAIL = "alice@e2e.test"
BOB_EMAIL = "bob@e2e.test"
CAROL_EMAIL = "carol@e2e.test"
DUP_EMAIL = "dup@e2e.test"
HIDDEN_EMAIL = "hidden@e2e.test"
EVE_EMAIL = "eve@e2e.test"
UNKNOWN_EMAIL = "nobody@e2e.test"

# person -> (email, account id, display_name, department, job_title)
PEOPLE: dict[uuid.UUID, tuple[str, str, str, str, str]] = {
    ALICE: (ALICE_EMAIL, "acc-alice", "Alice Admin", "Engineering", "CTO"),
    BOB: (BOB_EMAIL, "acc-bob", "Bob Builder", "Engineering", "Team Lead"),
    CAROL: (CAROL_EMAIL, "acc-carol", "Carol Coder", "Engineering", "Engineer"),
    DUP1: (DUP_EMAIL, "acc-dup1", "Dup One", "Sales", "AE"),
    DUP2: (DUP_EMAIL, "acc-dup2", "Dup Two", "Sales", "AE"),
    HIDDEN: (HIDDEN_EMAIL, "acc-hidden", "Hidden Hermit", "Finance", "CFO"),
}

# child -> parent (None = top of tree). All within TEST_TENANT_ID.
ORG_EDGES: dict[uuid.UUID, uuid.UUID | None] = {
    ALICE: None,
    BOB: ALICE,
    CAROL: BOB,
    DUP1: ALICE,
    DUP2: ALICE,
    HIDDEN: None,
}


def _connection(cfg: SessionConfig) -> pymysql.connections.Connection:
    return pymysql.connect(
        host=cfg.mariadb_host,
        port=cfg.mariadb_port,
        user=cfg.mariadb_user,
        password=cfg.mariadb_password,
        database=IDENTITY_DATABASE,
        charset="utf8mb4",
        autocommit=True,
    )


def _observation_rows(
    tenant: uuid.UUID, person: uuid.UUID, email: str, account: str, name: str, dept: str, title: str
) -> list[tuple]:
    """(value_type, value_id, value_full_text) observation triples for one person."""
    rows: list[tuple[str, str | None, str | None]] = [
        ("email", email, None),
        ("id", account, None),
        ("display_name", None, name),
        ("department", None, dept),
        ("job_title", None, title),
        ("status", None, "Active"),
    ]
    return [
        (value_type, SOURCE_TYPE, SOURCE_ID.bytes, tenant.bytes, value_id, value_full_text, person.bytes, ALICE.bytes)
        for (value_type, value_id, value_full_text) in rows
    ]


def seed(cfg: SessionConfig) -> None:
    """Insert the fixture dataset (idempotent: wipes e2e-seed rows first)."""
    conn = _connection(cfg)
    try:
        with conn.cursor() as cur:
            # Idempotent re-seed for local re-runs against a kept stack.
            for table in ("visibility", "person_roles", "org_chart", "account_person_map", "persons"):
                cur.execute(f"DELETE FROM {table} WHERE reason = 'e2e-seed'")  # noqa: S608 — fixed table names

            observation_sql = (
                "INSERT INTO persons (value_type, insight_source_type, insight_source_id,"
                " insight_tenant_id, value_id, value_full_text, person_id, author_person_id, reason)"
                " VALUES (%s, %s, %s, %s, %s, %s, %s, %s, 'e2e-seed')"
            )
            for person, (email, account, name, dept, title) in PEOPLE.items():
                cur.executemany(
                    observation_sql,
                    _observation_rows(TEST_TENANT_ID, person, email, account, name, dept, title),
                )
            # eve: same shape, other tenant.
            cur.executemany(
                observation_sql,
                _observation_rows(OTHER_TENANT, EVE, EVE_EMAIL, "acc-eve", "Eve Else", "Legal", "Counsel"),
            )

            org_sql = (
                "INSERT INTO org_chart (insight_tenant_id, insight_source_type, insight_source_id,"
                " child_person_id, parent_person_id, author_person_id, reason, valid_from, valid_to)"
                " VALUES (%s, %s, %s, %s, %s, %s, 'e2e-seed', UTC_TIMESTAMP(6), NULL)"
            )
            for child, parent in ORG_EDGES.items():
                cur.execute(
                    org_sql,
                    (
                        TEST_TENANT_ID.bytes,
                        SOURCE_TYPE,
                        SOURCE_ID.bytes,
                        child.bytes,
                        parent.bytes if parent else None,
                        ALICE.bytes,
                    ),
                )
            cur.execute(
                org_sql,
                (OTHER_TENANT.bytes, SOURCE_TYPE, SOURCE_ID.bytes, EVE.bytes, None, ALICE.bytes),
            )

            # alice is the tenant admin (fixed assignment id so revoke tests
            # elsewhere can reference it).
            cur.execute(
                "INSERT INTO person_roles (person_role_id, insight_tenant_id, person_id, role_id,"
                " valid_from, valid_to, author_person_id, reason)"
                " VALUES (%s, %s, %s, %s, UTC_TIMESTAMP(6), NULL, %s, 'e2e-seed')",
                (
                    ALICE_ADMIN_ASSIGNMENT.bytes,
                    TEST_TENANT_ID.bytes,
                    ALICE.bytes,
                    ADMIN_ROLE_ID.bytes,
                    ALICE.bytes,
                ),
            )

            # persons-seed operator: an active admin assignment in SEED_TENANT
            # (the admin gate reads person_roles only — no person row needed).
            cur.execute(
                "INSERT INTO person_roles (person_role_id, insight_tenant_id, person_id, role_id,"
                " valid_from, valid_to, author_person_id, reason)"
                " VALUES (%s, %s, %s, %s, UTC_TIMESTAMP(6), NULL, %s, 'e2e-seed')",
                (
                    SEED_ADMIN_ASSIGNMENT.bytes,
                    SEED_TENANT.bytes,
                    SEED_ADMIN.bytes,
                    ADMIN_ROLE_ID.bytes,
                    SEED_ADMIN.bytes,
                ),
            )

            # Explicit grant: bob may see hidden (alice may NOT — roles ≠ visibility).
            cur.execute(
                "INSERT INTO visibility (visibility_id, insight_tenant_id, viewer_person_id,"
                " viewed_person_id, valid_from, valid_to, author_person_id, reason)"
                " VALUES (%s, %s, %s, %s, UTC_TIMESTAMP(6), NULL, %s, 'e2e-seed')",
                (
                    VISIBILITY_GRANT_BOB_HIDDEN.bytes,
                    TEST_TENANT_ID.bytes,
                    BOB.bytes,
                    HIDDEN.bytes,
                    ALICE.bytes,
                ),
            )
    finally:
        conn.close()
    LOG.info("identity fixture dataset seeded into `%s`", IDENTITY_DATABASE)
