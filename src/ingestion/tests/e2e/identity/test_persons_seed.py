"""Contract: the persons-seed write path — POST /v1/persons-seed + the
operation-tracking reads.

The seed streams ClickHouse `identity.identity_inputs` and rebuilds the
caller-tenant's persons / account_person_map / org_chart. It runs here under
its own SEED_TENANT (see lib/identity_seed.py) so the rebuild never touches
the fixture tree the read tests depend on. The module fixture provisions the
`identity.identity_inputs` table with a deterministic three-account roster:
two accounts sharing an email (one person, two bindings) + one solo account.

The end-to-end case (a COMPLETED seed verified through the read path) runs
only where the implementation's ClickHouse reader works against the
harness's containerized ClickHouse — see
`lib.identity.supports_containerized_clickhouse`: the frozen .NET service's
Octonica native-protocol handshake deadlocks against every containerized CH
tried (works against the dev cluster's), so on `dotnet` that ONE case skips;
the Rust implementation (HTTP ClickHouse client) runs it — and that is the
run that matters as cutover acceptance. The other tests in this module only
need operations to EXIST (queued/running is fine), so they run on both
implementations and keep the coverage gate green.
"""

from __future__ import annotations

import time
import uuid

import pytest

from identity.contract import items_of
from lib import clickhouse
from lib import identity_seed as seed
from lib.config import SessionConfig

pytestmark = [pytest.mark.identity, pytest.mark.mutating]

SEED_SOURCE_ID = uuid.UUID("55555555-5555-5555-5555-555555555555")
SHARED_EMAIL = "seeded.person@e2e.test"
SOLO_EMAIL = "solo.person@e2e.test"

_OPERATION_TIMEOUT_S = 120.0


@pytest.fixture(scope="module")
def identity_inputs(compose_stack: SessionConfig):
    """Create + fill `identity.identity_inputs` (schema mirrors the dbt model's
    reader-relevant columns; extra dbt bookkeeping columns included so the
    service's `SELECT` never meets a missing column)."""
    clickhouse.ensure_database(compose_stack, "identity")
    clickhouse.execute(
        compose_stack,
        """
        CREATE TABLE IF NOT EXISTS identity.identity_inputs (
            unique_key          String,
            insight_tenant_id   Nullable(String),
            insight_source_type String,
            insight_source_id   Nullable(String),
            source_account_id   Nullable(String),
            value_type          Nullable(String),
            value               Nullable(String),
            operation_type      String,
            _synced_at          DateTime64(3, 'UTC'),
            _version            UInt64
        ) ENGINE = ReplacingMergeTree(_version) ORDER BY unique_key
        """,
    )
    clickhouse.execute(compose_stack, "TRUNCATE TABLE identity.identity_inputs")

    rows: list[tuple[str, str, str, str]] = [
        # (account, value_type, value) — two accounts share SHARED_EMAIL.
        ("seed-acc-1", "email", SHARED_EMAIL),
        ("seed-acc-1", "display_name", "Seeded Person"),
        ("seed-acc-2", "email", SHARED_EMAIL),
        ("seed-acc-3", "email", SOLO_EMAIL),
        ("seed-acc-3", "display_name", "Solo Person"),
    ]
    values = []
    for i, (account, value_type, value) in enumerate(rows):
        values.append(
            "("
            f"'{account}:{value_type}', "
            f"'{seed.SEED_TENANT}', 'e2e-source', '{SEED_SOURCE_ID}', "
            f"'{account}', '{value_type}', '{value}', "
            f"'UPSERT', now64(3), {i + 1}"
            ")"
        )
    clickhouse.execute(
        compose_stack,
        "INSERT INTO identity.identity_inputs "  # noqa: S608 — every value is a fixed test literal above, no untrusted input
        "(unique_key, insight_tenant_id, insight_source_type, insight_source_id,"
        " source_account_id, value_type, value, operation_type, _synced_at, _version) VALUES "
        + ", ".join(values),
    )
    return compose_stack


@pytest.fixture
def seed_api(identity_svc):
    """Client authenticated as the SEED_TENANT admin (see identity_seed)."""
    with identity_svc.client(sub=str(seed.SEED_ADMIN), tenant=str(seed.SEED_TENANT)) as c:
        yield c


@pytest.fixture
def seed_operation(identity_inputs, seed_api) -> str:
    """A freshly created seed operation's id — each dependent test owns its
    own operation instead of leaning on another test having run first."""
    r = seed_api.post("/v1/persons-seed", json={"mode": "link-by-email"})
    assert r.status_code == 202, f"status={r.status_code} body={r.text}"
    return r.json()["operation_id"]


def _wait_completed(client, operation_id: str) -> dict:
    deadline = time.monotonic() + _OPERATION_TIMEOUT_S
    last: dict = {}
    while time.monotonic() < deadline:
        r = client.get(f"/v1/persons-seed/{operation_id}")
        assert r.status_code == 200, f"status={r.status_code} body={r.text}"
        last = r.json()
        if last.get("status") in {"completed", "failed"}:
            return last
        time.sleep(0.5)
    raise AssertionError(f"seed operation did not finish in {_OPERATION_TIMEOUT_S:.0f}s: {last}")


def test_persons_seed_end_to_end(identity_inputs, seed_api, identity_svc) -> None:
    """202 + Location → operation completes → the seeded person resolves,
    with BOTH same-email accounts bound to one person."""
    if not identity_svc.supports_containerized_clickhouse:
        pytest.skip(
            "the .NET Octonica reader deadlocks against the harness's "
            "containerized ClickHouse (see module docstring); the Rust "
            "implementation runs this case"
        )
    r = seed_api.post("/v1/persons-seed", json={"mode": "link-by-email"})
    assert r.status_code == 202, f"status={r.status_code} body={r.text}"
    operation_id = r.json()["operation_id"]
    assert r.headers.get("location"), r.headers

    op = _wait_completed(seed_api, operation_id)
    assert op["status"] == "completed", op
    summary = op.get("summary") or {}
    assert summary, op

    # The two same-email accounts collapsed into ONE person...
    resolved = seed_api.post("/v1/profiles", json={"value_type": "email", "value": SHARED_EMAIL})
    assert resolved.status_code == 200, f"status={resolved.status_code} body={resolved.text}"
    person = resolved.json()
    accounts = {entry["value"] for entry in person.get("ids") or []}
    assert accounts == {"seed-acc-1", "seed-acc-2"}, person.get("ids")

    # ...and the solo account minted its own.
    solo = seed_api.post("/v1/profiles", json={"value_type": "email", "value": SOLO_EMAIL})
    assert solo.status_code == 200, f"status={solo.status_code} body={solo.text}"
    assert solo.json()["person_id"] != person["person_id"]


def test_persons_seed_operations_listed(seed_operation, seed_api) -> None:
    """The list carries the operation THIS test created — order-independent,
    green on a fresh database, no reliance on the end-to-end test."""
    r = seed_api.get("/v1/persons-seed")
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    ops = items_of(r.json())
    matching = [op for op in ops if op["operation_id"] == seed_operation]
    assert len(matching) == 1, ops
    assert matching[0]["operation_type"] == "persons-seed", matching[0]
    assert matching[0]["insight_tenant_id"] == str(seed.SEED_TENANT), matching[0]


def test_persons_seed_list_limit(seed_operation, seed_api) -> None:
    """With at least two operations present (the fixture's + one more),
    limit=1 returns exactly one — an empty list would mean the filter is
    vacuously 'passing'."""
    second = seed_api.post("/v1/persons-seed", json={"mode": "link-by-email"})
    assert second.status_code == 202, f"status={second.status_code} body={second.text}"
    r = seed_api.get("/v1/persons-seed?limit=1")
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    rows = items_of(r.json())
    assert len(rows) == 1, rows
    assert rows[0]["insight_tenant_id"] == str(seed.SEED_TENANT), rows[0]


def test_persons_seed_list_status_filter(seed_operation, seed_api) -> None:
    """The status filter includes the created operation under its current
    status and excludes it under a status it can no longer hold.

    The lifecycle is one-way (queued → running → completed|failed), so the
    inclusion check retries until a status read and the filtered list agree
    (the operation may transition between the two GETs — on a fast CI worker
    it can cross two states in milliseconds), and the exclusion check uses
    `queued`, which the operation can never re-enter once it was observed
    past it. No terminal state is required — the worker may legitimately
    still be running (or, on macOS Docker Desktop, stuck — see the module
    docstring)."""
    deadline = time.monotonic() + 30.0
    while True:
        r = seed_api.get(f"/v1/persons-seed/{seed_operation}")
        assert r.status_code == 200, f"status={r.status_code} body={r.text}"
        current = r.json()["status"]
        included = items_of(seed_api.get(f"/v1/persons-seed?status={current}").json())
        if seed_operation in {op["operation_id"] for op in included}:
            break
        assert time.monotonic() < deadline, (
            f"status read and ?status= filter never agreed within 30s "
            f"(last read: {current}; filtered: {included})"
        )
        time.sleep(0.2)
    assert all(op["status"] == current for op in included), included

    if current != "queued":
        # One-way lifecycle: once past `queued` it can never be queued again,
        # so this exclusion cannot race with a transition.
        excluded = items_of(seed_api.get("/v1/persons-seed?status=queued").json())
        assert seed_operation not in {op["operation_id"] for op in excluded}, excluded


def test_persons_seed_403_non_admin(bob_api) -> None:
    """bob is not an admin anywhere — the seed trigger is refused."""
    r = bob_api.post("/v1/persons-seed", json={"mode": "link-by-email"})
    assert r.status_code == 403, f"status={r.status_code} body={r.text}"


def test_persons_seed_401_unauthenticated(anon_api) -> None:
    assert anon_api.post("/v1/persons-seed", json={"mode": "link-by-email"}).status_code == 401
