"""Idempotency + roster-scope tests for `identity.seed_login_ids`.

A stdlib `unittest` test against a minimal fake cursor, locking two
regressions:

1. Idempotency: migration 004 (`004_persons_relax_constraints.sql`) put
   `created_at` in `persons`' unique key, so `INSERT IGNORE` alone no longer
   dedupes a re-run (each insert gets a fresh `created_at`, so the unique key
   never collides). Every writer must check for an existing row explicitly.
2. Roster scope: the Keycloak realm seeds the WHOLE roster (`keycloak_realm`
   pins every realm user's id to their own roster uuid) — `seed_login_ids`
   must seed a row per roster member, not just the dev lead.

Run against the installed package (see the README's develop section):

    uv run --extra dev python -m unittest discover -s tests -t .
"""

from __future__ import annotations

import os
import unittest
from typing import Any

from insight_seed import identity, profiles

_TENANT = "00000000-df51-5b42-9538-d2b56b7ee953"


def _roster() -> list[profiles.Person]:
    return [
        profiles.Person(
            uuid=profiles.DEV_LEAD_UUID,
            email="dev@company.nonpresent",
            team="development",
            role="lead",
            parent_uuid=profiles.CEO_UUID,
            first_name="Dev",
            last_name="Lead",
        ),
        profiles.Person(
            uuid=profiles.SALES_LEAD_UUID,
            email="sales-lead@company.nonpresent",
            team="sales",
            role="lead",
            parent_uuid=profiles.CEO_UUID,
            first_name="Sales",
            last_name="Lead",
        ),
    ]


class _FakeCursor:
    """Tracks which (person_id, external_id) pairs have been inserted.

    `seed_login_ids` runs one SELECT-then-maybe-INSERT per roster pair; this
    fake extracts the identifying `(person, external_id)` elements from each
    statement's params (they sit at different positions in the SELECT vs.
    INSERT param tuples — see `seed_login_ids`' `exists_sql`/`insert_sql`) so
    it can answer `fetchone()` per pair, not just a single global flag.
    """

    def __init__(self) -> None:
        self.insert_count = 0
        self.rowcount = 0
        self._existing: set[tuple[Any, Any]] = set()
        self._pending_result: tuple[int] | None = None

    def execute(self, sql: str, params: tuple[Any, ...] = ()) -> None:
        statement = sql.strip().upper()
        if statement.startswith("SELECT"):
            # exists_sql params: (tenant, person, source_type, source_id, external_id)
            key = (params[1], params[4])
            self._pending_result = (1,) if key in self._existing else None
        elif statement.startswith("INSERT"):
            # insert_sql params: (source_type, source_id, tenant, external_id, person, author, reason)
            key = (params[4], params[3])
            self._existing.add(key)
            self.insert_count += 1
            self.rowcount = 1
        else:
            raise AssertionError(f"unexpected SQL in seed_login_ids: {sql}")

    def fetchone(self) -> tuple[int] | None:
        return self._pending_result


class SeedLoginIdsTests(unittest.TestCase):
    """IDP_SOURCE_TYPE is SET, not defaulted: `profiles` reads it at call time,
    so a value left over from the developer's shell would otherwise decide which
    source_type these tests expect."""

    def setUp(self) -> None:
        self._previous = os.environ.get("IDP_SOURCE_TYPE")
        os.environ["IDP_SOURCE_TYPE"] = "keycloak"

    def tearDown(self) -> None:
        if self._previous is None:
            os.environ.pop("IDP_SOURCE_TYPE", None)
        else:
            os.environ["IDP_SOURCE_TYPE"] = self._previous

    def test_whole_roster_is_seeded_once_across_two_runs(self) -> None:
        cur = _FakeCursor()
        roster = _roster()

        # The fake cursor implements only what these writers call.
        first_run_count = identity.seed_login_ids(cur, _TENANT, roster)  # type: ignore[arg-type]
        second_run_count = identity.seed_login_ids(cur, _TENANT, roster)  # type: ignore[arg-type]

        self.assertEqual(
            first_run_count,
            len(roster),
            "every roster persona gets a login row (keycloak_realm registers all of them)",
        )
        self.assertEqual(second_run_count, 0, "re-run must be a no-op for every pair")
        self.assertEqual(cur.insert_count, len(roster))


if __name__ == "__main__":
    unittest.main()


class _ObservationCursor:
    """Answers the exists-then-insert pair the observation writers issue.

    Keyed on (person, value_type, value) — the logical identity of an
    observation, which is exactly what `created_at` stopped enforcing. The two
    writers order their parameters differently, so the INSERT key is read from
    the statement's own shape.
    """

    def __init__(self) -> None:
        self.insert_count = 0
        self.rowcount = 0
        self._seen: set[tuple[Any, Any, Any]] = set()
        self._pending: tuple[int] | None = None

    def execute(self, sql: str, params: tuple[Any, ...] = ()) -> None:
        head = sql.strip().upper()
        if head.startswith("SELECT"):
            # _observation_exists: (tenant, person, source_type, source_id, value_type, value)
            self._pending = (1,) if (params[1], params[4], params[5]) in self._seen else None
        elif head.startswith("INSERT"):
            if "value_full_text" in sql:
                # (value_type, source_type, source_id, tenant, value, person, author, reason)
                key = (params[5], params[0], params[4])
            else:
                # value_type is the literal 'email' in the statement:
                # (source_type, source_id, tenant, value_id, person, author, reason)
                key = (params[4], "email", params[3])
            self._seen.add(key)
            self.insert_count += 1
            self.rowcount = 1
        else:
            raise AssertionError(f"unexpected SQL: {sql}")

    def fetchone(self) -> tuple[int] | None:
        return self._pending


class SeedPersonsIdempotencyTests(unittest.TestCase):
    """The writers that used to rely on `INSERT IGNORE`. Migration 004 put
    `created_at` in the unique key, so a re-run never collided and every run
    appended a duplicate observation; each writer now checks first."""

    def setUp(self) -> None:
        self._previous = os.environ.get("IDP_SOURCE_TYPE")
        os.environ["IDP_SOURCE_TYPE"] = "keycloak"

    def tearDown(self) -> None:
        if self._previous is None:
            os.environ.pop("IDP_SOURCE_TYPE", None)
        else:
            os.environ["IDP_SOURCE_TYPE"] = self._previous

    def test_seed_persons_inserts_once_across_two_runs(self) -> None:
        cur = _ObservationCursor()
        roster = _roster()

        first = identity.seed_persons(cur, _TENANT, roster)  # type: ignore[arg-type]
        second = identity.seed_persons(cur, _TENANT, roster)  # type: ignore[arg-type]

        self.assertEqual(first, len(roster), "the first run writes one row per person")
        self.assertEqual(second, 0, "a re-run must be a no-op, not a duplicate observation")
        self.assertEqual(cur.insert_count, len(roster))

    def test_seed_person_names_inserts_once_across_two_runs(self) -> None:
        cur = _ObservationCursor()
        roster = _roster()

        first = identity.seed_person_names(cur, _TENANT, roster)  # type: ignore[arg-type]
        second = identity.seed_person_names(cur, _TENANT, roster)  # type: ignore[arg-type]

        self.assertGreater(first, 0, "names are written on the first run")
        self.assertEqual(second, 0, "a re-run must be a no-op, not a duplicate observation")
        self.assertEqual(cur.insert_count, first)
