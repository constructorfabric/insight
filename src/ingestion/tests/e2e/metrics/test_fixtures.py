from __future__ import annotations

import logging
import re
from typing import Any

import pytest
from lib import clickhouse
from lib.analytics import AnalyticsProcess
from lib.ch_seeder import CHSeeder
from lib.config import SessionConfig
from lib.dbt_runner import DbtRunner
from lib.enrich import EnrichRunner
from lib.expect_engine import evaluate_case
from lib.fixture_loader import IdentityAccount, TestYaml
from lib.identity_stub import IdentityStub, person_id_for
from lib.worker import WorkerContext

pytestmark = pytest.mark.fixture
LOG = logging.getLogger("e2e.runner")


_EMAIL_TOKEN = re.compile(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}")


def _requested_persona_emails(test_yaml: TestYaml) -> list[str]:
    """Emails the yaml's metric-results cases address (`entity.ids`) — the
    set translated to person UUIDs on the wire."""
    emails: list[str] = []
    for case in test_yaml.cases:
        request = case.get("request") or {}
        if not str(request.get("url", "")).endswith("/v1/metric-results"):
            continue
        ids = ((request.get("body") or {}).get("entity") or {}).get("ids") or []
        emails.extend(i for i in ids if isinstance(i, str) and "@" in i)
    return sorted(set(emails))


def _all_persona_emails(test_yaml: TestYaml) -> list[str]:
    """Every email the yaml mentions anywhere in its bronze seeds, plus the
    requested ones.

    ALL of them get identity_persons bindings — not just the requested ids:
    peer pools are built from cohort members (HR emails seeded as data, never
    requested), and an unbound member would resolve to NULL and silently
    shrink the pool the expects count.
    """
    emails = set(_requested_persona_emails(test_yaml))

    def walk(value: Any) -> None:
        if isinstance(value, str):
            emails.update(_EMAIL_TOKEN.findall(value))
        elif isinstance(value, list):
            for item in value:
                walk(item)
        elif isinstance(value, dict):
            for item in value.values():
                walk(item)

    walk(test_yaml.bronze)
    return sorted(emails)


def _person_ids_for(emails: list[str], aliases: dict[str, list[str]]) -> dict[str, str]:
    """email -> person id, with `identity_aliases` bound to the canonical
    persona's id so several source accounts resolve to ONE person."""
    canonical_of = {alias.strip().lower(): canonical for canonical, group in aliases.items() for alias in group}
    return {email: person_id_for(canonical_of.get(email.strip().lower(), email)) for email in emails}


def _seed_identity_persons(cfg: SessionConfig, person_ids: dict[str, str]) -> None:
    """Give each persona an account that carries their email and is bound to
    their person id — the shape resolve_person_id() reads.

    Resolution is account-derived: `identity_inputs` says which account carries
    an email, `identity_persons` says who that account belongs to. The rig plays
    both producers (the connector models and the service's persons-sync), one
    synthetic account per persona.

    Runs BEFORE the gold dbt build so every observation row resolves.
    """
    # NOT worker-scoped: the resolve_person_id macro names `identity` literally,
    # so a per-worker suffix here would leave gold reading an unseeded table.
    # Enabling xdist for this suite has to make the macro schema-aware first.
    clickhouse.ensure_database(cfg, "identity")
    clickhouse.execute(
        cfg,
        """
        CREATE TABLE IF NOT EXISTS identity.identity_persons (
            id UInt64, value_type String,
            insight_source_type String, insight_source_id UUID,
            insight_tenant_id UUID,
            value_id Nullable(String), value_full_text Nullable(String),
            value Nullable(String), value_effective Nullable(String),
            person_id UUID, author_person_id UUID, reason Nullable(String),
            created_at DateTime64(6, 'UTC'), _synced_at DateTime64(3, 'UTC')
        ) ENGINE = MergeTree ORDER BY id
        """,
    )
    clickhouse.execute(
        cfg,
        """
        CREATE TABLE IF NOT EXISTS identity.identity_inputs (
            unique_key String, insight_tenant_id UUID,
            insight_source_id UUID, insight_source_type String,
            source_account_id Nullable(String), value_type String,
            value Nullable(String), value_field_name String,
            operation_type String, _synced_at DateTime64(3), _version Int64
        ) ENGINE = ReplacingMergeTree(_version) ORDER BY unique_key
        SETTINGS allow_nullable_key = 1
        """,
    )
    clickhouse.execute(cfg, "TRUNCATE TABLE identity.identity_persons")
    clickhouse.execute(cfg, "TRUNCATE TABLE identity.identity_inputs")
    if not person_ids:
        return

    personas = sorted(person_ids.items())

    # The account id is the email: deterministic, and the two tables must agree
    # on it for the join to land.
    binding_rows = ", ".join(
        f"({index + 1}, 'id', 'e2e-rig', toUUID('{_RIG_SOURCE_ID}'), generateUUIDv4(), "
        f"'{email}', '{email}', toUUID('{person_id}'), "
        f"toUUID('00000000-0000-0000-0000-000000000000'), now64(6), now64(3))"
        for index, (email, person_id) in enumerate(personas)
    )
    clickhouse.execute(
        cfg,
        "INSERT INTO identity.identity_persons "  # noqa: S608 — values derive from fixture emails
        "(id, value_type, insight_source_type, insight_source_id, insight_tenant_id,"
        " value_id, value_effective, person_id, author_person_id, created_at, _synced_at) "
        "VALUES " + binding_rows,
    )

    evidence_rows = ", ".join(
        f"('e2e-rig:{email}:email', generateUUIDv4(), toUUID('{_RIG_SOURCE_ID}'), 'e2e-rig', "
        f"'{email}', 'email', '{email}', 'e2e.rig.email', 'UPSERT', now64(3), 1)"
        for email, _ in personas
    )
    clickhouse.execute(
        cfg,
        "INSERT INTO identity.identity_inputs "  # noqa: S608 — values derive from fixture emails
        "(unique_key, insight_tenant_id, insight_source_id, insight_source_type,"
        " source_account_id, value_type, value, value_field_name, operation_type,"
        " _synced_at, _version) "
        "VALUES " + evidence_rows,
    )


# The reserved not-a-human person (bots, CI): an account bound to it claims
# nothing in either resolution map. Mirrors excluded_person_id() in dbt.
_EXCLUDED_PERSON_ID = "ffffffff-ffff-ffff-ffff-ffffffffffff"


def _seed_identity_accounts(cfg: SessionConfig, accounts: list[IdentityAccount], start_id: int) -> None:
    """Source-account bindings from the yaml's `identity_accounts` — the rows
    the account-first map (resolve_person_id_by_account) resolves pull-request
    authors through. insight_source_id is hashed from the RAW source id with
    the same expression the connectors' identity_inputs models use, so the
    seam under test is the real one.
    """
    if not accounts:
        return

    rows = ", ".join(
        f"({start_id + index}, 'id', '{entry.source_type}', "
        f"toUUID(UUIDNumToString(sipHash128('{entry.source_id}'))), generateUUIDv4(), "
        f"'{entry.account_id}', '{entry.account_id}', "
        f"toUUID('{_EXCLUDED_PERSON_ID if entry.person == 'excluded' else person_id_for(entry.person)}'), "
        f"toUUID('00000000-0000-0000-0000-000000000000'), now64(6), now64(3))"
        for index, entry in enumerate(accounts)
    )
    clickhouse.execute(
        cfg,
        "INSERT INTO identity.identity_persons "  # noqa: S608 — values derive from fixture yaml
        "(id, value_type, insight_source_type, insight_source_id, insight_tenant_id,"
        " value_id, value_effective, person_id, author_person_id, created_at, _synced_at) "
        "VALUES " + rows,
    )


# One synthetic connector instance for every rig persona: the account triple is
# what joins evidence to bindings, so both inserts must name the same source.
_RIG_SOURCE_ID = "e2e0e2e0-0000-4000-8000-000000000001"


def _translate(value: Any, mapping: dict[str, str]) -> Any:
    """Deep-copy `value` swapping every mapped string (request emails →
    person UUIDs, or response UUIDs → emails for the expect rules)."""
    if isinstance(value, str):
        return mapping.get(value, value)
    if isinstance(value, list):
        return [_translate(item, mapping) for item in value]
    if isinstance(value, dict):
        return {key: _translate(item, mapping) for key, item in value.items()}
    return value


def test_metric_smoke(
    test_yaml: TestYaml,
    ch_seeder: CHSeeder,
    dbt_runner: DbtRunner,
    enrich_runner: EnrichRunner,
    analytics: AnalyticsProcess,
    identity_stub: IdentityStub,
    worker_ctx: WorkerContext,
) -> None:
    ch_seeder.truncate_touched()

    # 2. Seed this test's resolved bronze records.
    ch_seeder.seed_bronze(test_yaml.bronze, test_yaml.schemas)

    # 3. Build the dbt models the seeded tables feed: staging first (the `+`
    #    pulls <connector>__bronze_promoted), then the silver class models.
    staging, silver = dbt_runner.derive_selectors(test_yaml.touched_tables)
    if staging:
        # Record staging models in the ledger BEFORE building. They live in the
        # `staging` schema and are read by the silver models via union_by_tag, so a
        # prior test's staging rows (e.g. dates this test doesn't re-seed) would
        # survive into the silver rebuild and contaminate later tests' gold-view
        # aggregates. Recording up front (not after) means a build that raises
        # partway still leaves the table in the truncate ledger so the next test
        # cleans it; recording a model that never materialised is harmless
        # (truncate_touched uses TRUNCATE TABLE IF EXISTS).
        for st in staging:
            ch_seeder.ledger.record("staging", st)
        dbt_runner.build(" ".join(f"+{m}" for m in staging), worker_ctx=worker_ctx)
    # 3b. Connector enrich steps (descriptor.images.enrich), between staging and
    #     silver — mirrors prod: dbt(tag:<c>) → <c>-enrich → dbt(silver). Data-driven
    #     from descriptors, so any connector with an enrich step participates (jira
    #     today, youtrack once it ships one). The enrich binary reads the connector's
    #     staging tables (built above) and writes back into `staging.*`.
    touched_schemas = {schema for schema, _ in test_yaml.touched_tables}
    ran_enrich_steps = []
    for step in enrich_runner.steps_for(touched_schemas):
        source_ids = enrich_runner.discover_source_ids(step, test_yaml.touched_tables)
        if not source_ids:
            continue
        # The enrich binary APPENDS into its staging output tables, and dbt never
        # rebuilds them (they are sources, not models), so a prior test's rows for
        # the same source_id would survive into this test's silver rebuild and
        # inflate absolute-count metrics. Clear them before enriching so each test
        # starts from a clean enrich output (the silver class table read from them
        # is already truncated via the ledger above).
        for schema, table in dbt_runner.enrich_output_tables(step.name):
            ch_seeder.truncate_table(schema, table)
        enrich_runner.run(step, source_ids)
        ran_enrich_steps.append(step)

    # 3c. Silver class models. Build exactly what the seeded data supports:
    #     derive_selectors gives the silver fed by seeded bronze (e.g. class_task_users,
    #     class_task_field_metadata); each enrich step additionally feeds silver via an
    #     EPHEMERAL staging view (e.g. class_task_field_history), which derive_selectors
    #     can't see. We build that precise set BY NAME rather than the connector's broad
    #     `tag:silver,tag:<c>+` so unseeded streams (class_task_sprints, the identity
    #     chain, …) are not dragged in and fail on absent bronze. Only steps that
    #     ACTUALLY ran (had a source_id) contribute their ephemeral targets — otherwise
    #     we'd build silver that depends on enrich output that was never produced.
    silver_set = set(silver)
    silver_set.discard("identity_inputs")
    for step in ran_enrich_steps:
        silver_set.update(dbt_runner.ephemeral_silver_targets(step.name))
    run_only_silver = silver_set & {"class_hr_working_hours"}
    tested_silver = silver_set - run_only_silver
    if tested_silver:
        # Record before building (same rationale as staging above): a build that
        # raises partway still leaves the targets in the truncate ledger for the
        # next test to clean.
        for cls in tested_silver:
            ch_seeder.ledger.record("silver", cls)
        dbt_runner.build(" ".join(sorted(tested_silver)), worker_ctx=worker_ctx)
    if run_only_silver:
        for cls in run_only_silver:
            ch_seeder.ledger.record("silver", cls)
        dbt_runner.run(" ".join(sorted(run_only_silver)), worker_ctx=worker_ctx)
    if "class_collab_meeting_activity" in silver_set:
        ch_seeder.ledger.record("silver", "class_focus_metrics")
        dbt_runner.run("class_focus_metrics", worker_ctx=worker_ctx, full_refresh=True)

    # 4. Identity bindings for the personas the cases address, BEFORE the
    #    gold build — the resolve macro joins them into person_id during the
    #    build (the rig plays the persons-sync role here).
    persona_emails = _all_persona_emails(test_yaml)
    all_person_ids = _person_ids_for(persona_emails, test_yaml.identity_aliases)
    to_person_id = {email: all_person_ids[email] for email in _requested_persona_emails(test_yaml)}
    # The visibility gate asks the stub about the ids this case requests, so
    # the stub's visible set is derived from the yaml — never a hand-kept list
    # a new persona could fall outside of (that reads as an authz bug).
    identity_stub.allow_visible(persona_emails)
    _seed_identity_persons(ch_seeder.cfg, all_person_ids)
    _seed_identity_accounts(ch_seeder.cfg, test_yaml.identity_accounts, start_id=len(all_person_ids) + 1)

    if staging or silver_set or ran_enrich_steps:
        dbt_runner.run("tag:gold", worker_ctx=worker_ctx)

    # 5. Run each case's request and evaluate its expect rules. The yaml
    #    speaks emails (the persona key); the wire speaks person UUIDs since
    #    the identity cutover — translate on the way out and back so the 36
    #    case files stay human-readable.
    canonical_of = {
        alias.strip().lower(): canonical for canonical, group in test_yaml.identity_aliases.items() for alias in group
    }
    to_email: dict[str, str] = {}
    for email, person_id in to_person_id.items():
        seen = to_email.get(person_id)
        if seen is None:
            to_email[person_id] = email
            continue
        # uuid5 is injective over distinct inputs, so an UNDECLARED collision
        # means two spellings of one email (case) — the reverse map would drop
        # one silently and the expects would fail somewhere unrelated. Declared
        # aliases share an id on purpose; the canonical spelling wins, so the
        # expects can name one person.
        canonical = canonical_of.get(email.lower()) or canonical_of.get(seen.lower())
        if canonical is None:
            raise AssertionError(f"two requested spellings share a person id: {seen!r} and {email!r}")
        to_email[person_id] = canonical
    for case in test_yaml.cases:
        status, payload = analytics.call_request(_translate(case["request"], to_person_id))
        if status != 200:
            LOG.warning("HTTP %d; body: %r", status, payload)
        evaluate_case(case, _translate(payload, to_email), status)
