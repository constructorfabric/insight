"""Versatility gaps flagged in #1602's QA review of the persons-seed org-chart
projection (#1690): every case in `test_persons_seed.py` proves the projection
against identity_inputs rows the fixture INSERTs by hand — the real
connector -> dbt -> `identity.identity_inputs` path is never exercised, and
several BR-8/BR-9 shapes (arbitrary depth, cycles, a second HR source) have no
test at all. This module closes those gaps one at a time.
"""

from __future__ import annotations

import uuid
from pathlib import Path

import pytest
import yaml
from lib import clickhouse
from lib import identity_seed as seed
from lib.ch_seeder import CHSeeder
from lib.config import SessionConfig
from lib.dbt_runner import DbtRunner
from lib.worker import WorkerContext

pytestmark = [pytest.mark.identity, pytest.mark.mutating]

_SCHEMAS_PATH = Path(__file__).parents[1] / "metrics" / "schemas" / "bronze_bamboohr.employees.yaml"
_BAMBOOHR_SCHEMAS = yaml.safe_load(_SCHEMAS_PATH.read_text(encoding="utf-8"))["schemas"]

# No `schemas/bronze_ms_entra.users.yaml` fixture exists yet (the metrics rig
# has never needed to seed this table) — declared here from
# `scripts/connectors-ddl/ms-entra.sql`, which is itself the deliberate proof
# point of the test below: that DDL carries no manager/managerDn column at all.
_MS_ENTRA_SCHEMAS = {
    "bronze_ms_entra.users": {
        "properties": {
            "_airbyte_raw_id": {"type": "string"},
            "_airbyte_extracted_at": {"type": "string", "format": "date-time"},
            "_airbyte_meta": {"type": "string"},
            "_airbyte_generation_id": {"type": "integer"},
            "id": {"type": ["string", "null"]},
            "userPrincipalName": {"type": ["string", "null"]},
            "mail": {"type": ["string", "null"]},
            "displayName": {"type": ["string", "null"]},
            "employeeId": {"type": ["string", "null"]},
            "department": {"type": ["string", "null"]},
            "jobTitle": {"type": ["string", "null"]},
            "accountEnabled": {"type": ["boolean", "null"]},
            "onPremisesSamAccountName": {"type": ["string", "null"]},
            "userType": {"type": ["string", "null"]},
            "tenant_id": {"type": ["string", "null"]},
            "source_id": {"type": ["string", "null"]},
            "unique_key": {"type": ["string", "null"]},
        }
    }
}


def _ms_entra_user(*, run_tag: str, entity_id: str, email: str) -> dict:
    """A minimal `bronze_ms_entra.users` row — deliberately has no manager
    field of any kind, since the real DDL has none to give one.

    Only `mail` is populated — of ALL eleven columns `ms_entra__users_snapshot`
    tracks (not just the five `ms_entra__identity_inputs.sql` reads), since
    the shared `identity_inputs_from_history` macro's ADR-0002 `id_upserts`
    CTE emits one canonical `id` row per TRACKED-FIELD history row for the
    entity (unfiltered by which fields identity_inputs actually reads), not
    one per entity. Seeding a second non-null field in the same snapshot
    batch collides on `unique_key` (every field's version-1 row shares the
    same `updated_at`) — a separate latent issue in the shared macro, not
    what this test is about.
    """
    return {
        "_airbyte_raw_id": str(uuid.uuid4()),
        "_airbyte_extracted_at": "2026-01-05T00:00:00",
        "_airbyte_meta": "{}",
        "_airbyte_generation_id": 0,
        "id": entity_id,
        "unique_key": f"pipeline-{run_tag}-ms-entra-{entity_id}",
        "tenant_id": f"pipeline-tenant-{run_tag}",
        "source_id": f"pipeline-source-{run_tag}",
        "mail": email,
    }


def _bamboohr_employee(
    *, run_tag: str, entity_id: str, email: str, display_name: str, supervisor_email: str | None
) -> dict:
    """A minimal `bronze_bamboohr.employees` row — the real shape the bamboohr
    connector would append, not a hand-crafted identity_inputs row."""
    return {
        # Non-nullable Airbyte CDK columns — real connector rows always carry
        # these; some staging transformations (e.g. latest-row selection)
        # rely on `_airbyte_extracted_at`.
        "_airbyte_raw_id": str(uuid.uuid4()),
        "_airbyte_extracted_at": "2026-01-05T00:00:00",
        "_airbyte_meta": "{}",
        "_airbyte_generation_id": 0,
        "id": entity_id,
        "unique_key": f"pipeline-{run_tag}-bamboohr-{entity_id}",
        "tenant_id": f"pipeline-tenant-{run_tag}",
        "source_id": f"pipeline-source-{run_tag}",
        "workEmail": email,
        "displayName": display_name,
        "firstName": display_name.split(" ")[0],
        "lastName": display_name.split(" ")[-1],
        "employeeNumber": entity_id,
        "jobTitle": "Engineer",
        "department": "Engineering",
        "division": "Engineering",
        "status": "Active",
        "supervisorEmail": supervisor_email,
        "supervisorEId": None,
    }


def _person_id_by_email(identity_svc, email: str) -> str:
    with identity_svc.client(
        sub=str(seed.SEED_ADMIN), tenant=str(seed.SEED_TENANT), sub_type="service", roles="service"
    ) as svc:
        r = svc.get("/internal/persons/by-email-override", params={"email": email})
        assert r.status_code == 200, f"status={r.status_code} body={r.text}"
        return r.json()["insight_source_id"]


def _open_parent(cfg: SessionConfig, child: str) -> str | None:
    """The single OPEN (valid_to IS NULL) org_chart parent for `child`, under
    SEED_TENANT. Raw SQL on purpose — this asserts the seed's WRITE, mirroring
    `test_persons_seed.py::_org_chart_edges`."""
    with seed._connection(cfg) as conn, conn.cursor() as cur:  # noqa: SLF001 — harness-internal helper
        cur.execute(
            "SELECT LOWER(HEX(parent_person_id))"
            " FROM org_chart"
            " WHERE insight_tenant_id = %s AND child_person_id = %s AND valid_to IS NULL",
            (seed.SEED_TENANT.bytes, uuid.UUID(child).bytes),
        )
        rows = cur.fetchall()
        assert len(rows) <= 1, f"expected at most one open edge for {child}, got {rows}"
        return rows[0][0] if rows else None


def _hex(person: str) -> str:
    return uuid.UUID(person).hex


def _ensure_identity_inputs_table(cfg: SessionConfig) -> None:
    """`identity.identity_inputs` schema mirrors the dbt model's reader-relevant
    columns (see `test_persons_seed.py::identity_inputs`). CREATE TABLE IF NOT
    EXISTS so this is order-independent with that fixture and with the real
    dbt-built table from the connector-pipeline test above."""
    clickhouse.ensure_database(cfg, "identity")
    clickhouse.execute(
        cfg,
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


def _insert_raw_inputs(
    cfg: SessionConfig,
    rows: list[tuple[str, str, str]],
    *,
    run_tag: str,
    version_start: int,
    source_type: str = "e2e-cycle-source",
) -> None:
    """Hand-insert observation rows into identity_inputs — same shape as
    `test_persons_seed.py::_insert_inputs`, kept local so this module does not
    depend on test collection order in another file.

    `source_type` defaults to a harness-only value — fine for write-side-only
    assertions (raw SQL on org_chart), but `GET /v1/subchart` filters by the
    rig's CONFIGURED `org_chart_source_type` (`bamboohr`, see lib/identity.py)
    and silently returns nothing for any other source, so a test that reads
    through that endpoint must pass `source_type="bamboohr"`.
    """
    values = []
    for i, (account, value_type, value) in enumerate(rows):
        values.append(
            "("
            f"'{run_tag}:{account}:{value_type}:{version_start + i}', "
            f"'{seed.SEED_TENANT}', '{source_type}', '77777777-7777-7777-7777-777777777777', "
            f"'{account}', '{value_type}', '{value}', "
            f"'UPSERT', now64(3) - INTERVAL {len(rows) - i} SECOND, "
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


def test_seed_org_chart_from_real_bamboohr_connector_pipeline(
    identity_svc,
    ch_seeder: CHSeeder,
    dbt_runner: DbtRunner,
    worker_ctx: WorkerContext,
    compose_stack: SessionConfig,
) -> None:
    """The org_chart projection holds when `identity.identity_inputs` is
    populated through the REAL path (bronze -> bamboohr connector dbt models ->
    the shared `identity_inputs` union), not a hand-inserted row. Every other
    org-chart test in this suite bypasses that path entirely; this is the one
    proof that the bamboohr connector's own dbt models actually feed the seed.
    """
    if not identity_svc.supports_seed_cli:
        pytest.skip("the seed CLI exists only on the Rust implementation (#1690)")

    run_tag = uuid.uuid4().hex[:10]
    manager_email = f"pipeline.manager.{run_tag}@e2e.test"
    report_email = f"pipeline.report.{run_tag}@e2e.test"

    ch_seeder.truncate_touched()
    ch_seeder.seed_bronze(
        {
            "bronze_bamboohr.employees": [
                _bamboohr_employee(
                    run_tag=run_tag,
                    entity_id=f"mgr-{run_tag}",
                    email=manager_email,
                    display_name="Pipeline Manager",
                    supervisor_email=None,
                ),
                _bamboohr_employee(
                    run_tag=run_tag,
                    entity_id=f"rep-{run_tag}",
                    email=report_email,
                    display_name="Pipeline Report",
                    supervisor_email=manager_email,
                ),
            ]
        },
        _BAMBOOHR_SCHEMAS,
    )

    staging, silver = dbt_runner.derive_selectors({("bronze_bamboohr", "employees")})
    dbt_runner.build(" ".join(f"+{m}" for m in staging), worker_ctx=worker_ctx)
    assert "identity_inputs" in silver, (
        f"bamboohr__identity_inputs did not surface a silver:identity_inputs tag (silver={silver}) "
        "— derive_selectors no longer sees the connector's identity path"
    )
    dbt_runner.run("identity_inputs", worker_ctx=worker_ctx)

    landed = clickhouse.query(
        compose_stack,
        "SELECT count() FROM identity.identity_inputs"
        f" WHERE insight_source_type = 'bamboohr' AND source_account_id = 'rep-{run_tag}'"
        "   AND value_type = 'parent_email'",
    )
    assert landed[0][0] >= 1, (
        "the bamboohr connector's own dbt models never produced a parent_email row in "
        "identity.identity_inputs for the seeded report — the connector/dbt path is broken, "
        "not just untested"
    )

    res = identity_svc.run_seed_cli(tenant=str(seed.SEED_TENANT), force=True)
    assert res.returncode == 0, f"rc={res.returncode}\n{res.stdout}\n{res.stderr}"

    manager = _person_id_by_email(identity_svc, manager_email)
    report = _person_id_by_email(identity_svc, report_email)
    assert _open_parent(compose_stack, report) == _hex(manager)


def test_seed_and_subchart_survive_a_circular_manager_chain(identity_svc, compose_stack: SessionConfig) -> None:
    """A→B and B→A both resolvable: an explicit #1602 target with no test
    anywhere in the suite. The seed's WRITE side stores org_chart edges
    verbatim from parent_email observations (it does not detect cycles — see
    `domain/subchart.rs`'s comment: cycle-safety is a READ-side concern), so
    both directions must land as open edges. The READ side must not hang or
    error: `subchart.rs` caps every subtree query at the server's configured
    `max_depth` specifically so a cyclic org_chart terminates instead of
    recursing forever (`WITH RECURSIVE ... UNION ALL`, unlike the `UNION`/
    distinct visibility CTE, does not self-terminate on a cycle)."""
    if not identity_svc.supports_seed_cli:
        pytest.skip("the seed CLI exists only on the Rust implementation (#1690)")

    run_tag = uuid.uuid4().hex[:10]
    a_email = f"cycle.a.{run_tag}@e2e.test"
    b_email = f"cycle.b.{run_tag}@e2e.test"

    _ensure_identity_inputs_table(compose_stack)
    _insert_raw_inputs(
        compose_stack,
        [
            (f"cycle-a-{run_tag}", "email", a_email),
            (f"cycle-a-{run_tag}", "id", f"cycle-a-{run_tag}"),
            (f"cycle-a-{run_tag}", "parent_email", b_email),
            (f"cycle-b-{run_tag}", "email", b_email),
            (f"cycle-b-{run_tag}", "id", f"cycle-b-{run_tag}"),
            (f"cycle-b-{run_tag}", "parent_email", a_email),
        ],
        run_tag=run_tag,
        version_start=1,
        # GET /v1/subchart below only traverses org_chart rows matching the
        # rig's configured org_chart_source_type ("bamboohr") — any other
        # value makes the read return an empty (root-only) tree regardless of
        # what got written, silently defeating the read-side assertion.
        source_type="bamboohr",
    )

    res = identity_svc.run_seed_cli(tenant=str(seed.SEED_TENANT), force=True)
    assert res.returncode == 0, f"the seed must terminate on cyclic input, not hang or crash\nrc={res.returncode}\n{res.stdout}\n{res.stderr}"

    person_a = _person_id_by_email(identity_svc, a_email)
    person_b = _person_id_by_email(identity_svc, b_email)

    # Both directions of the cycle are stored — the write side is a verbatim
    # projection of the observations, not a cycle-breaking algorithm.
    assert _open_parent(compose_stack, person_a) == _hex(person_b)
    assert _open_parent(compose_stack, person_b) == _hex(person_a)

    with identity_svc.client(sub=person_a, tenant=str(seed.SEED_TENANT)) as api:
        r = api.get(f"/v1/subchart/{person_a}")
    assert r.status_code == 200, f"cyclic org_chart must not 500/hang the subchart read: status={r.status_code} body={r.text}"

    depths: list[int] = []

    def _walk(node: dict, depth: int) -> None:
        depths.append(depth)
        for child in node.get("subordinates", []):
            _walk(child, depth + 1)

    _walk(r.json()["root"], 0)
    # Must actually descend the cycle (proves the recursion ran, not just
    # that the endpoint returned something), yet stay bounded by the harness's
    # configured max_depth=16 (lib/identity.py) — a true infinite loop would
    # never reach this assertion at all.
    assert 2 <= max(depths) <= 16, f"subchart descent on a cycle was not bounded as expected: {depths}"


def test_ms_entra_connector_emits_no_org_chart_signal_yet(
    ch_seeder: CHSeeder,
    dbt_runner: DbtRunner,
    worker_ctx: WorkerContext,
    compose_stack: SessionConfig,
) -> None:
    """Documents a real product gap, not just a test gap: `#1602`'s QA review
    flagged "MS Entra has no org-chart test at all", but the reason is that
    `ms_entra__identity_inputs.sql` tracks no manager field whatsoever —
    `scripts/connectors-ddl/ms-entra.sql` has no `managerDn`/`manager` column
    to read one from. Active Directory has a dedicated
    `active_directory__manager_identity_inputs.sql` that resolves `managerDn`
    to a manager's email; MS Entra has no equivalent, so there is no
    org_chart-relevant signal to seed a real org-chart test against.

    This seeds one user through the REAL ms-entra connector/dbt path and pins
    the current (missing) behavior: no `parent_email`/`parent_id` observation
    is ever produced. When manager sync is added for MS Entra, this assertion
    fails — replace it with a real org-chart projection test mirroring
    `test_seed_org_chart_from_real_bamboohr_connector_pipeline` above.
    """
    run_tag = uuid.uuid4().hex[:10]
    entity_id = f"entra-{run_tag}"
    email = f"entra.user.{run_tag}@e2e.test"

    ch_seeder.truncate_touched()
    ch_seeder.seed_bronze(
        {"bronze_ms_entra.users": [_ms_entra_user(run_tag=run_tag, entity_id=entity_id, email=email)]},
        _MS_ENTRA_SCHEMAS,
    )

    staging, silver = dbt_runner.derive_selectors({("bronze_ms_entra", "users")})
    dbt_runner.build(" ".join(f"+{m}" for m in staging), worker_ctx=worker_ctx)
    assert "identity_inputs" in silver, f"ms_entra__identity_inputs no longer tags silver:identity_inputs (silver={silver})"
    dbt_runner.run("identity_inputs", worker_ctx=worker_ctx)

    emitted = clickhouse.query(
        compose_stack,
        "SELECT DISTINCT value_type FROM identity.identity_inputs"
        f" WHERE insight_source_type = 'ms-entra' AND source_account_id = '{entity_id}'",
    )
    value_types = {row[0] for row in emitted}
    assert value_types, "expected at least id/email/display_name identity signals from the seeded user"
    assert "parent_email" not in value_types and "parent_id" not in value_types, (
        f"ms-entra now emits a manager signal ({value_types}) — the #1602 gap is closed. "
        "Rewrite this test into a real org-chart projection proof (mirroring "
        "active_directory__manager_identity_inputs.sql) instead of asserting absence."
    )


def test_seed_and_subchart_project_arbitrary_depth_from_a_synced_chain(
    identity_svc,
    ch_seeder: CHSeeder,
    dbt_runner: DbtRunner,
    worker_ctx: WorkerContext,
    compose_stack: SessionConfig,
) -> None:
    """BR-8 ("any depth") is asserted elsewhere only against `test_subchart.py`'s
    hardcoded two-level fixture tree (alice -> bob -> carol), never against a
    chain that actually flowed through a connector. Seed a five-level
    `supervisorEmail` chain (deeper than the fixed fixture) into
    `bronze_bamboohr.employees`, build the REAL bamboohr connector/dbt models
    plus the shared `identity_inputs` union — same path as
    `test_seed_org_chart_from_real_bamboohr_connector_pipeline` — then prove
    both write (org_chart parent-per-level) and read (GET /v1/subchart
    depth-per-level) reflect the full chain, not a depth the seed or the API
    silently caps at 2."""
    if not identity_svc.supports_seed_cli:
        pytest.skip("the seed CLI exists only on the Rust implementation (#1690)")

    run_tag = uuid.uuid4().hex[:10]
    chain_len = 5  # deeper than the fixed fixture's 2 levels; well under max_depth=16
    emails = [f"chain.{i}.{run_tag}@e2e.test" for i in range(chain_len)]

    ch_seeder.truncate_touched()
    ch_seeder.seed_bronze(
        {
            "bronze_bamboohr.employees": [
                _bamboohr_employee(
                    run_tag=run_tag,
                    entity_id=f"chain-{i}-{run_tag}",
                    email=emails[i],
                    display_name=f"Chain Level {i}",
                    supervisor_email=emails[i - 1] if i > 0 else None,
                )
                for i in range(chain_len)
            ]
        },
        _BAMBOOHR_SCHEMAS,
    )

    staging, silver = dbt_runner.derive_selectors({("bronze_bamboohr", "employees")})
    dbt_runner.build(" ".join(f"+{m}" for m in staging), worker_ctx=worker_ctx)
    assert "identity_inputs" in silver, f"bamboohr__identity_inputs did not surface a silver:identity_inputs tag (silver={silver})"
    dbt_runner.run("identity_inputs", worker_ctx=worker_ctx)

    res = identity_svc.run_seed_cli(tenant=str(seed.SEED_TENANT), force=True)
    assert res.returncode == 0, f"rc={res.returncode}\n{res.stdout}\n{res.stderr}"

    person_ids = [_person_id_by_email(identity_svc, email) for email in emails]

    # Write side: every level's OPEN parent is exactly the level above it.
    for i in range(1, chain_len):
        assert _open_parent(compose_stack, person_ids[i]) == _hex(person_ids[i - 1]), (
            f"level {i} did not project a parent edge to level {i - 1}"
        )
    assert _open_parent(compose_stack, person_ids[0]) is None, "the root of the chain must have no open parent"

    # Read side: the subtree rooted at the top must nest all `chain_len`
    # levels — a depth cap baked into the seed or the API would truncate this.
    with identity_svc.client(sub=person_ids[0], tenant=str(seed.SEED_TENANT)) as api:
        r = api.get(f"/v1/subchart/{person_ids[0]}", params={"depth": chain_len})
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"

    depth_by_person: dict[str, int] = {}

    def _walk(node: dict, depth: int) -> None:
        depth_by_person[node["person_id"]] = depth
        for child in node.get("subordinates", []):
            _walk(child, depth + 1)

    _walk(r.json()["root"], 0)
    for i, person_id in enumerate(person_ids):
        assert depth_by_person.get(person_id) == i, (
            f"level {i} ({person_id}) surfaced at depth {depth_by_person.get(person_id)!r} in the subchart, expected {i}"
        )
