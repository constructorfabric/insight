"""Contract: the persons-seed write path + the operation-tracking reads.

The seed streams ClickHouse `identity.identity_inputs` and rebuilds the
caller-tenant's persons / account_person_map / org_chart. It runs here under
its own SEED_TENANT (see lib/identity_seed.py) so the rebuild never touches
the fixture tree the read tests depend on. The module fixture provisions the
`identity.identity_inputs` table with a deterministic three-account roster:
two accounts sharing an email (one person, two bindings) + one solo account.

TRIGGER DIVERGENCE (#1690, accepted): the .NET service triggers the seed via
`POST /v1/persons-seed` (async queue + poll); the Rust successor REMOVED the
POST — the seed is CLI-only there (`identity-resolution seed`, run by the
Helm CronJob / a manual Job; synchronous) and only the GET journal routes
remain. Tests select the trigger through `_trigger_seed` and gate the
POST-specific cases on `supports_seed_http_trigger`; the CLI-specific cases
(input guards, advisory lock, exit codes) gate on `supports_seed_cli`. The
POST cases die with the .NET service.

The end-to-end case runs only where the implementation's ClickHouse reader
works against the harness's containerized ClickHouse — see
`lib.identity.supports_containerized_clickhouse`: the frozen .NET service's
Octonica native-protocol handshake deadlocks against every containerized CH
tried, so on `dotnet` that ONE case skips; the Rust implementation (HTTP
ClickHouse client) runs it — and that is the run that matters as cutover
acceptance.
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
BOSS_EMAIL = "boss.person@e2e.test"
# A parent_email no account ever carries as its email — the seed must skip
# the edge (ADR-0010: no stub persons) and fall back to a NULL-parent
# membership row.
GHOST_EMAIL = "ghost.manager@e2e.test"

# A tenant nothing ever seeds successfully: `persons` never has rows under it,
# so the wrong-tenant guard must always refuse an unforced run for it.
GUARD_TENANT = uuid.UUID("66666666-6666-6666-6666-666666666666")

# Author the CLI stamps on its journal rows (no JWT on that path) — the
# SYSTEM_AUTHOR nil-UUID convention shared with the legacy Python seed.
SYSTEM_AUTHOR = "00000000-0000-0000-0000-000000000000"

_OPERATION_TIMEOUT_S = 120.0

_ROSTER: list[tuple[str, str, str]] = [
    # (account, value_type, value) — two accounts share SHARED_EMAIL.
    # Connectors emit a source-native `id` observation per account; the
    # profile's `ids[]` is built from exactly those. The parent_email rows
    # give the org-chart rebuild something to derive edges from: the shared
    # person reports to the boss; the solo person's manager is unresolvable
    # (GHOST_EMAIL belongs to nobody); the boss reports to nobody.
    ("seed-boss", "email", BOSS_EMAIL),
    ("seed-boss", "id", "seed-boss"),
    ("seed-acc-1", "email", SHARED_EMAIL),
    ("seed-acc-1", "id", "seed-acc-1"),
    ("seed-acc-1", "display_name", "Seeded Person"),
    ("seed-acc-1", "parent_email", BOSS_EMAIL),
    ("seed-acc-2", "email", SHARED_EMAIL),
    ("seed-acc-2", "id", "seed-acc-2"),
    ("seed-acc-3", "email", SOLO_EMAIL),
    ("seed-acc-3", "id", "seed-acc-3"),
    ("seed-acc-3", "display_name", "Solo Person"),
    ("seed-acc-3", "parent_email", GHOST_EMAIL),
]


def _insert_inputs(
    cfg: SessionConfig,
    rows: list[tuple[str, str, str]],
    version_start: int,
    shift_seconds: int = 0,
) -> None:
    """INSERT observation rows into identity_inputs, one distinct _synced_at
    per row (production reality): the seed derives observation created_at
    from it, and the persons UNIQUE key (…, value_type, created_at) silently
    drops same-instant collisions — e.g. two accounts' `id` observations for
    the same person.

    `shift_seconds` nudges the batch AFTER earlier same-run batches (the
    manager-change rows must outrank the roster). Keep it SMALL (seconds):
    the MariaDB `persons` log survives sessions on a kept stack (the session
    seed wipes only reason='e2e-seed' rows), so an input stamped far in the
    future outranks the NEXT session's fresh roster in the latest-observation
    race until the wall clock catches up — an order-of-runs flake.
    """
    values = []
    for i, (account, value_type, value) in enumerate(rows):
        offset = len(rows) - i
        values.append(
            "("
            f"'{account}:{value_type}:{version_start + i}', "
            f"'{seed.SEED_TENANT}', 'e2e-source', '{SEED_SOURCE_ID}', "
            f"'{account}', '{value_type}', '{value}', "
            f"'UPSERT', now64(3) + INTERVAL {shift_seconds} SECOND - INTERVAL {offset} SECOND, "
            f"{version_start + i}"
            ")"
        )
    clickhouse.execute(
        cfg,
        "INSERT INTO identity.identity_inputs "  # noqa: S608 — every value is a fixed test literal above, no untrusted input
        "(unique_key, insight_tenant_id, insight_source_type, insight_source_id,"
        " source_account_id, value_type, value, operation_type, _synced_at, _version) VALUES "
        + ", ".join(values),
    )


def _fill_roster(cfg: SessionConfig) -> None:
    """TRUNCATE + INSERT the deterministic roster into identity_inputs."""
    clickhouse.execute(cfg, "TRUNCATE TABLE identity.identity_inputs")
    _insert_inputs(cfg, _ROSTER, 1)


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
    _fill_roster(compose_stack)
    return compose_stack


@pytest.fixture
def seed_api(identity_svc):
    """Client authenticated as the SEED_TENANT admin (see identity_seed)."""
    with identity_svc.client(sub=str(seed.SEED_ADMIN), tenant=str(seed.SEED_TENANT)) as c:
        yield c


def _trigger_seed(identity_svc, seed_api) -> str:
    """Trigger one seed run through the implementation's trigger and return
    its operation id. POST (async) on .NET; the `seed` CLI on Rust —
    synchronous, so the returned operation is already terminal there.

    The CLI run is `--force`: the fixture dataset lives under TEST_TENANT_ID
    while the seed runs under SEED_TENANT, which is exactly the wrong-tenant
    shape the guard exists for — the guard's own contract is proven by the
    unforced tests below.
    """
    if identity_svc.supports_seed_http_trigger:
        r = seed_api.post("/v1/persons-seed", json={"mode": "link-by-email"})
        assert r.status_code == 202, f"status={r.status_code} body={r.text}"
        return r.json()["operation_id"]
    res = identity_svc.run_seed_cli(tenant=str(seed.SEED_TENANT), force=True)
    assert res.returncode == 0, f"rc={res.returncode}\n{res.stdout}\n{res.stderr}"
    r = seed_api.get("/v1/persons-seed?limit=1")
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    rows = items_of(r.json())
    assert rows, "a completed CLI run must be visible in the journal"
    return rows[0]["operation_id"]


@pytest.fixture
def seed_operation(identity_inputs, seed_api, identity_svc) -> str:
    """A freshly created seed operation's id — each dependent test owns its
    own operation instead of leaning on another test having run first."""
    return _trigger_seed(identity_svc, seed_api)


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
    """Seed run → operation completes → the seeded person resolves,
    with BOTH same-email accounts bound to one person."""
    if not identity_svc.supports_containerized_clickhouse:
        pytest.skip(
            "the .NET Octonica reader deadlocks against the harness's "
            "containerized ClickHouse (see module docstring); the Rust "
            "implementation runs this case"
        )
    operation_id = _trigger_seed(identity_svc, seed_api)

    op = _wait_completed(seed_api, operation_id)
    assert op["status"] == "completed", op
    summary = op.get("summary") or {}
    assert summary, op
    if identity_svc.supports_seed_cli:
        # CLI journal contract: system author (no JWT on that path) and the
        # request records the trigger.
        assert op["author_person_id"] == SYSTEM_AUTHOR, op
        request = op.get("request") or {}
        assert request.get("trigger") == "cli", op

    # Freshly minted person_ids come from the tenant-agnostic internal lookup
    # (no visibility gate — at this point NOBODY is in the seed admin's
    # subtree, so /v1/profiles would correctly answer 404 for the admin).
    with identity_svc.client(
        sub=str(seed.SEED_ADMIN), tenant=str(seed.SEED_TENANT), sub_type="service", roles="service"
    ) as svc:
        shared = svc.get(f"/internal/persons/by-email/{SHARED_EMAIL}")
        assert shared.status_code == 200, f"status={shared.status_code} body={shared.text}"
        shared_id = shared.json()["insight_source_id"]
        solo = svc.get(f"/internal/persons/by-email/{SOLO_EMAIL}")
        assert solo.status_code == 200, f"status={solo.status_code} body={solo.text}"
        # The solo account minted its own person.
        assert solo.json()["insight_source_id"] != shared_id

    # The two same-email accounts collapsed into ONE person — proven through
    # the read contract, resolved AS that person (a seeded top-of-tree sees
    # itself): the shared email resolves to a single profile (NOT ambiguous),
    # and the by-id/`ids[]` surface carries the CURRENT id observation per
    # source instance (the rn=1 reduction both implementations share) — for
    # two accounts in one source that is the newest one, seed-acc-2.
    with identity_svc.client(sub=shared_id, tenant=str(seed.SEED_TENANT)) as pc:
        by_email = pc.post("/v1/profiles", json={"value_type": "email", "value": SHARED_EMAIL})
        assert by_email.status_code == 200, f"status={by_email.status_code} body={by_email.text}"
        person = by_email.json()
        assert person["person_id"] == shared_id, person
        accounts = {entry["value"] for entry in person.get("ids") or []}
        assert accounts == {"seed-acc-2"}, person.get("ids")

        by_id = pc.post(
            "/v1/profiles",
            json={
                "value_type": "id",
                "value": "seed-acc-2",
                "insight_source_type": "e2e-source",
                "insight_source_id": str(SEED_SOURCE_ID),
            },
        )
        assert by_id.status_code == 200, f"status={by_id.status_code} body={by_id.text}"
        assert by_id.json()["person_id"] == shared_id, by_id.json()


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


def test_persons_seed_list_limit(seed_operation, seed_api, identity_svc) -> None:
    """With at least two operations present (the fixture's + one more),
    limit=1 returns exactly one — an empty list would mean the filter is
    vacuously 'passing'."""
    _trigger_seed(identity_svc, seed_api)
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
    it can cross two states in milliseconds; a CLI-triggered run is terminal
    already), and the exclusion check uses `queued`, which the operation can
    never re-enter once it was observed past it. No terminal state is
    required — the .NET worker may legitimately still be running (or, on
    macOS Docker Desktop, stuck — see the module docstring)."""
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


# ── POST trigger (dotnet-only; dies with the .NET service) ────────────────


def test_persons_seed_403_non_admin(bob_api, identity_svc) -> None:
    """bob is not an admin anywhere — the seed trigger is refused."""
    if not identity_svc.supports_seed_http_trigger:
        pytest.skip("POST /v1/persons-seed removed in the Rust successor (#1690)")
    r = bob_api.post("/v1/persons-seed", json={"mode": "link-by-email"})
    assert r.status_code == 403, f"status={r.status_code} body={r.text}"


def test_persons_seed_401_unauthenticated(anon_api, identity_svc) -> None:
    if not identity_svc.supports_seed_http_trigger:
        pytest.skip("POST /v1/persons-seed removed in the Rust successor (#1690)")
    assert anon_api.post("/v1/persons-seed", json={"mode": "link-by-email"}).status_code == 401


# ── CLI trigger (rust-only): guards, lock, exit codes (#1690) ─────────────


def _operation_row(cfg: SessionConfig, tenant: uuid.UUID) -> dict | None:
    """Newest `operations` row for a tenant, read straight from MariaDB —
    guard-refused tenants have no admin, so the HTTP journal is unreadable
    for them by design."""
    with seed._connection(cfg) as conn, conn.cursor() as cur:  # noqa: SLF001 — harness-internal helper
        cur.execute(
            "SELECT status, error_message, HEX(author_person_id) AS author"
            " FROM operations WHERE insight_tenant_id = %s"
            " ORDER BY started_at DESC LIMIT 1",
            (tenant.bytes,),
        )
        row = cur.fetchone()
    if row is None:
        return None
    if isinstance(row, dict):
        return row
    status, error_message, author = row
    return {"status": status, "error_message": error_message, "author": author}


def test_seed_cli_wrong_tenant_guard(identity_inputs, identity_svc, compose_stack) -> None:
    """An unforced run for a tenant `persons` has never seen — while other
    tenants' rows exist — must refuse (exit 3) and journal the refusal:
    seeding would mint a parallel person set under a wrong tenant
    (HOTFIX(#1550): the unfiltered reader re-files every row under the
    configured tenant)."""
    if not identity_svc.supports_seed_cli:
        pytest.skip("the seed CLI exists only on the Rust implementation (#1690)")
    res = identity_svc.run_seed_cli(tenant=str(GUARD_TENANT), force=False)
    assert res.returncode == 3, f"rc={res.returncode}\n{res.stdout}\n{res.stderr}"

    op = _operation_row(compose_stack, GUARD_TENANT)
    assert op is not None, "the guard refusal must still write a journal row"
    assert op["status"] == "failed", op
    assert "tenant" in (op["error_message"] or ""), op


def test_seed_cli_empty_input_guard(identity_inputs, identity_svc, compose_stack) -> None:
    """An unforced run over an EMPTY identity_inputs must refuse (exit 3) —
    an empty read means a broken/misconfigured pipeline, not 'no people'."""
    if not identity_svc.supports_seed_cli:
        pytest.skip("the seed CLI exists only on the Rust implementation (#1690)")
    clickhouse.execute(compose_stack, "TRUNCATE TABLE identity.identity_inputs")
    try:
        res = identity_svc.run_seed_cli(tenant=str(seed.SEED_TENANT), force=False)
        assert res.returncode == 3, f"rc={res.returncode}\n{res.stdout}\n{res.stderr}"
        op = _operation_row(compose_stack, seed.SEED_TENANT)
        assert op is not None and op["status"] == "failed", op
        assert "identity_inputs" in (op["error_message"] or ""), op
    finally:
        # The module fixture fills once (module scope) — restore for the
        # tests that run after this one.
        _fill_roster(compose_stack)


def test_seed_cli_unconfigured_tenant_refuses_when_ambiguous(
    identity_inputs, identity_svc, compose_stack
) -> None:
    """With NO tenant configured the binary may only infer a tenant when the
    persons log holds exactly one — the fixture dataset spans several
    (TEST_TENANT_ID, OTHER_TENANT, ...), so an unconfigured run must refuse
    (exit 1) instead of guessing one of them."""
    if not identity_svc.supports_seed_cli:
        pytest.skip("the seed CLI exists only on the Rust implementation (#1690)")
    res = identity_svc.run_seed_cli(tenant=None, force=True)
    assert res.returncode == 1, f"rc={res.returncode}\n{res.stdout}\n{res.stderr}"
    assert "ambiguous" in (res.stdout + res.stderr), res.stderr


def test_seed_cli_failure_exits_1_and_journals(identity_inputs, identity_svc, compose_stack) -> None:
    """A run that fails AFTER the journal row exists (here: unreachable
    ClickHouse) exits 1 and leaves a failed operation carrying only the
    generic message — raw driver/anyhow text must not leak to the journal,
    which the GET endpoints return verbatim."""
    if not identity_svc.supports_seed_cli:
        pytest.skip("the seed CLI exists only on the Rust implementation (#1690)")
    res = identity_svc.run_seed_cli(
        tenant=str(seed.SEED_TENANT),
        force=True,
        # Closed port → fast connection refusal on the identity_inputs read,
        # which happens after the operations row is enqueued.
        extra_env={"APP__gears__identity-resolution__config__clickhouse_url": "http://127.0.0.1:1"},
    )
    assert res.returncode == 1, f"rc={res.returncode}\n{res.stdout}\n{res.stderr}"

    op = _operation_row(compose_stack, seed.SEED_TENANT)
    assert op is not None and op["status"] == "failed", op
    assert op["error_message"] == "persons-seed failed; see job logs", op


def test_seed_cli_sweeps_zombie_operations(identity_inputs, identity_svc, compose_stack) -> None:
    """A killed Job pod leaves its operations row `running` with no process
    to resolve it — the next run's pre-seed sweep must fail rows older than
    the cutoff (1h) and leave fresh ones alone (they may be a live run)."""
    if not identity_svc.supports_seed_cli:
        pytest.skip("the seed CLI exists only on the Rust implementation (#1690)")
    stale = uuid.uuid4()
    fresh = uuid.uuid4()
    insert_sql = (
        "INSERT INTO operations (operation_id, operation_type, status,"
        " insight_tenant_id, author_person_id, started_at)"
        " VALUES (%s, 'persons-seed', 'running', %s, %s,"
        " UTC_TIMESTAMP(6) - INTERVAL %s MINUTE)"
    )
    with seed._connection(compose_stack) as conn, conn.cursor() as cur:  # noqa: SLF001 — harness-internal helper
        cur.execute(insert_sql, (stale.bytes, seed.SEED_TENANT.bytes, uuid.UUID(int=0).bytes, 120))
        cur.execute(insert_sql, (fresh.bytes, seed.SEED_TENANT.bytes, uuid.UUID(int=0).bytes, 5))
    try:
        res = identity_svc.run_seed_cli(tenant=str(seed.SEED_TENANT), force=True)
        assert res.returncode == 0, f"rc={res.returncode}\n{res.stdout}\n{res.stderr}"

        with seed._connection(compose_stack) as conn, conn.cursor() as cur:  # noqa: SLF001
            cur.execute(
                "SELECT LOWER(HEX(operation_id)), status, error_message FROM operations"
                " WHERE operation_id IN (%s, %s)",
                (stale.bytes, fresh.bytes),
            )
            rows = {op_id: (status, message) for op_id, status, message in cur.fetchall()}
        assert rows[stale.hex] == ("failed", "aborted by pod restart"), rows
        assert rows[fresh.hex][0] == "running", rows
    finally:
        # The synthetic rows have no process behind them — drop them so the
        # fresh one can't confuse later journal assertions.
        with seed._connection(compose_stack) as conn, conn.cursor() as cur:  # noqa: SLF001
            cur.execute(
                "DELETE FROM operations WHERE operation_id IN (%s, %s)",
                (stale.bytes, fresh.bytes),
            )


# ── input → org_chart correspondence (#1690: the projection itself) ───────


def _person_id_by_email(identity_svc, email: str) -> str:
    """Resolve a seeded person's UUID via the tenant-agnostic internal
    lookup (freshly minted persons are in nobody's visibility subtree)."""
    with identity_svc.client(
        sub=str(seed.SEED_ADMIN), tenant=str(seed.SEED_TENANT), sub_type="service", roles="service"
    ) as svc:
        r = svc.get(f"/internal/persons/by-email/{email}")
        assert r.status_code == 200, f"status={r.status_code} body={r.text}"
        return r.json()["insight_source_id"]


def _org_chart_edges(cfg: SessionConfig, child: str) -> list[tuple[str | None, bool]]:
    """(parent_person_id, is_open) org_chart rows for a child, oldest first.

    Straight SQL on purpose: this asserts the seed's WRITE (inputs →
    projection), not the read API — the profile projection filters by
    `org_chart_source_type` (bamboohr in the rig config), which the
    handcrafted fixture tree already covers in the read tests.
    """
    with seed._connection(cfg) as conn, conn.cursor() as cur:  # noqa: SLF001 — harness-internal helper
        cur.execute(
            "SELECT LOWER(HEX(parent_person_id)), valid_to IS NULL"
            " FROM org_chart"
            " WHERE insight_tenant_id = %s AND child_person_id = %s"
            " ORDER BY valid_from",
            (seed.SEED_TENANT.bytes, uuid.UUID(child).bytes),
        )
        return [(row[0], bool(row[1])) for row in cur.fetchall()]


def _hex(person: str) -> str:
    return uuid.UUID(person).hex


def test_seed_org_chart_matches_inputs(identity_inputs, identity_svc, compose_stack) -> None:
    """The org_chart the seed rebuilds corresponds to the parent_email
    observations in identity_inputs: a resolvable manager becomes the open
    edge, an unresolvable one degrades to a NULL-parent membership row
    (ADR-0010: no stub persons), and a top-of-tree person gets the Path-B
    NULL-parent row."""
    if not identity_svc.supports_seed_cli:
        pytest.skip("the seed CLI exists only on the Rust implementation (#1690)")
    res = identity_svc.run_seed_cli(tenant=str(seed.SEED_TENANT), force=True)
    assert res.returncode == 0, f"rc={res.returncode}\n{res.stdout}\n{res.stderr}"

    boss = _person_id_by_email(identity_svc, BOSS_EMAIL)
    shared = _person_id_by_email(identity_svc, SHARED_EMAIL)
    solo = _person_id_by_email(identity_svc, SOLO_EMAIL)

    # shared reports to boss: exactly one open edge, parent = boss.
    open_edges = [p for p, is_open in _org_chart_edges(compose_stack, shared) if is_open]
    assert open_edges == [_hex(boss)], open_edges

    # solo's manager is unresolvable (GHOST_EMAIL belongs to nobody) — the
    # edge is skipped, membership survives as a NULL-parent row.
    open_edges = [p for p, is_open in _org_chart_edges(compose_stack, solo) if is_open]
    assert open_edges == [None], open_edges

    # boss reports to nobody — Path-B NULL-parent membership row.
    open_edges = [p for p, is_open in _org_chart_edges(compose_stack, boss) if is_open]
    assert open_edges == [None], open_edges


def test_seed_manager_change_reaches_org_chart(identity_inputs, identity_svc, compose_stack) -> None:
    """THE #1690 regression: a manager change lands in identity_inputs → a
    RE-RUN of the seed moves the open org_chart edge to the new manager and
    closes the old one. Before the fix nothing re-ran the seed, so this
    exact transition never reached the Team view.

    The whole cast is PER-RUN unique (uuid-suffixed accounts/emails), never
    the shared roster: the MariaDB persons log outlives sessions on a kept
    stack, so a re-parenting of a REUSED account would poison the next
    session's roster in the latest-observation race whenever runs land
    seconds apart (an order-of-runs flake we hit). A fresh child has no
    cross-session history by construction.
    """
    if not identity_svc.supports_seed_cli:
        pytest.skip("the seed CLI exists only on the Rust implementation (#1690)")
    run_tag = uuid.uuid4().hex[:12]
    child_acc = f"flip-child-{run_tag}"
    child_email = f"flip.child.{run_tag}@e2e.test"
    boss_a_acc = f"flip-boss-a-{run_tag}"
    boss_a_email = f"flip.boss.a.{run_tag}@e2e.test"
    boss_b_acc = f"flip-boss-b-{run_tag}"
    boss_b_email = f"flip.boss.b.{run_tag}@e2e.test"

    # Baseline ingest: the child reports to manager A.
    _insert_inputs(
        compose_stack,
        [
            (boss_a_acc, "email", boss_a_email),
            (boss_a_acc, "id", boss_a_acc),
            (child_acc, "email", child_email),
            (child_acc, "id", child_acc),
            (child_acc, "parent_email", boss_a_email),
        ],
        version_start=100,  # distinct RMT _version space from the roster
        shift_seconds=4,  # newer than any roster row (max now-1)
    )
    res = identity_svc.run_seed_cli(tenant=str(seed.SEED_TENANT), force=True)
    assert res.returncode == 0, f"rc={res.returncode}\n{res.stdout}\n{res.stderr}"

    child = _person_id_by_email(identity_svc, child_email)
    boss_a = _person_id_by_email(identity_svc, boss_a_email)
    open_edges = [p for p, is_open in _org_chart_edges(compose_stack, child) if is_open]
    assert open_edges == [_hex(boss_a)], f"baseline edge must point at manager A: {open_edges}"

    # The connector ingests a manager change: a NEWER parent_email pointing
    # at a new (also newly ingested) manager B.
    _insert_inputs(
        compose_stack,
        [
            (boss_b_acc, "email", boss_b_email),
            (boss_b_acc, "id", boss_b_acc),
            (child_acc, "parent_email", boss_b_email),
        ],
        version_start=200,
        shift_seconds=8,  # newer than the baseline batch above
    )
    res = identity_svc.run_seed_cli(tenant=str(seed.SEED_TENANT), force=True)
    assert res.returncode == 0, f"rc={res.returncode}\n{res.stdout}\n{res.stderr}"

    boss_b = _person_id_by_email(identity_svc, boss_b_email)
    edges = _org_chart_edges(compose_stack, child)
    open_edges = [p for p, is_open in edges if is_open]
    assert open_edges == [_hex(boss_b)], f"open edge must move to manager B: {edges}"
    # Manager A's edge is closed, not erased — SCD2 history survives.
    assert (_hex(boss_a), False) in edges, edges


def test_seed_cli_lock_busy(identity_inputs, identity_svc, compose_stack) -> None:
    """A run against a held per-tenant advisory lock fails fast with exit 2 —
    the serialization that replaced the in-process queue (cron-vs-manual and
    multi-instance overlap)."""
    if not identity_svc.supports_seed_cli:
        pytest.skip("the seed CLI exists only on the Rust implementation (#1690)")
    with seed._connection(compose_stack) as conn, conn.cursor() as cur:  # noqa: SLF001 — harness-internal helper
        cur.execute("SELECT GET_LOCK(%s, 0)", (f"persons-seed:{seed.SEED_TENANT}",))
        got = cur.fetchone()
        assert got and next(iter(got if isinstance(got, tuple) else got.values())) == 1, got
        try:
            res = identity_svc.run_seed_cli(tenant=str(seed.SEED_TENANT), force=True)
            assert res.returncode == 2, f"rc={res.returncode}\n{res.stdout}\n{res.stderr}"
        finally:
            cur.execute("SELECT RELEASE_LOCK(%s)", (f"persons-seed:{seed.SEED_TENANT}",))
