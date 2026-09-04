"""One spec's data path, from seeded bronze to a caller that can read it.

Everything that ran once per spec in the generic YAML driver lives here: seed the
spec's bronze, build its staging and silver slice, run its enrich steps, publish its
people to identity, build gold, and hand back a `SpecRun` whose `call()` sends a
request as the spec wrote it -- addressing people by email -- and returns the response
with person ids translated back to emails, so a test asserts in the spec's own terms.
"""

from __future__ import annotations

import logging
import re
from dataclasses import dataclass, field
from typing import Any

from lib import clickhouse
from lib.analytics import AnalyticsProcess
from lib.ch_seeder import CHSeeder
from lib.config import SessionConfig
from lib.dbt_runner import DbtRunner
from lib.enrich import EnrichRunner
from lib.fixture_loader import IdentityAccount, TestYaml
from lib.identity_stub import IdentityStub, person_id_for
from lib.metric_expect import Ledger, MetricResponse
from lib.tracked_models import TrackedModels
from lib.worker import WorkerContext

LOG = logging.getLogger("e2e.spec")

_EMAIL_TOKEN = re.compile(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}")

_EXCLUDED_PERSON_ID = "ffffffff-ffff-ffff-ffff-ffffffffffff"

_RIG_SOURCE_ID = "e2e0e2e0-0000-4000-8000-000000000001"


def person_ids_for(emails: list[str], aliases: dict[str, list[str]]) -> dict[str, str]:
    """email -> person id, with `identity_aliases` bound to the canonical
    persona's id so several source accounts resolve to ONE person."""
    canonical_of = {alias.strip().lower(): canonical for canonical, group in aliases.items() for alias in group}
    return {email: person_id_for(canonical_of.get(email.strip().lower(), email)) for email in emails}


def seed_identity_persons(cfg: SessionConfig, person_ids: dict[str, str]) -> None:
    """Give each persona an account that carries their email and is bound to
    their person id — the shape `identity.person_map` resolves through.

    Resolution is account-derived: `identity_inputs` says which account carries
    an email, `identity_persons` says who that account belongs to. The rig plays
    both producers (the connector models and the service's persons-sync), one
    synthetic account per persona.

    Ordering is not free: the map RELATION must exist before the gold build,
    because `metric_entity_cohorts_current` is a view that INNER JOINs
    `person_map` and ClickHouse validates a view's query when it creates it. The
    map's CONTENTS are read per request, so only the rows may change afterwards.
    """
    # NOT worker-scoped: the map models name the `identity` schema literally, so
    # a per-worker suffix would leave the map reading an unseeded table. xdist
    # needs them schema-aware first.
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
        "INSERT INTO identity.identity_persons "
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
        "INSERT INTO identity.identity_inputs "
        "(unique_key, insight_tenant_id, insight_source_id, insight_source_type,"
        " source_account_id, value_type, value, value_field_name, operation_type,"
        " _synced_at, _version) "
        "VALUES " + evidence_rows,
    )


def seed_identity_accounts(cfg: SessionConfig, accounts: list[IdentityAccount], start_id: int) -> None:
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
        "INSERT INTO identity.identity_persons "
        "(id, value_type, insight_source_type, insight_source_id, insight_tenant_id,"
        " value_id, value_effective, person_id, author_person_id, created_at, _synced_at) "
        "VALUES " + rows,
    )


def translate(value: Any, mapping: dict[str, str]) -> Any:
    """Deep-copy `value` swapping every mapped string (request emails →
    person UUIDs, or response UUIDs → emails for the expect rules)."""
    if isinstance(value, str):
        return mapping.get(value, value)
    if isinstance(value, list):
        return [translate(item, mapping) for item in value]
    if isinstance(value, dict):
        return {key: translate(item, mapping) for key, item in value.items()}
    return value


def all_persona_emails(bronze: dict[str, list[dict[str, Any]]]) -> list[str]:
    """Every email the spec's bronze mentions. All of them are bound: peer pools are
    built from cohort members that are seeded as data and never requested, and an
    unbound member would resolve to NULL and silently shrink the pool."""
    found: set[str] = set()

    def walk(value: Any) -> None:
        if isinstance(value, str):
            found.update(_EMAIL_TOKEN.findall(value))
        elif isinstance(value, list):
            for item in value:
                walk(item)
        elif isinstance(value, dict):
            for item in value.values():
                walk(item)

    walk(bronze)
    return sorted(found)


def email_of_person(person_ids: dict[str, str], aliases: dict[str, list[str]]) -> dict[str, str]:
    """person id -> the one spelling a test names.

    `person_id_for` normalizes, so several bronze spellings of one address share a
    person id; a declared alias binds a different address to the same person. Both
    resolve here to the normalized canonical spelling, which is the one a test writes.
    """
    canonical_of = {alias.strip().lower(): canonical for canonical, group in aliases.items() for alias in group}
    return {
        person_id: canonical_of.get(email.strip().lower(), email.strip().lower())
        for email, person_id in sorted(person_ids.items())
    }


@dataclass
class SpecRun:
    """A spec's built data path and the caller that reads it."""

    spec: TestYaml
    analytics: AnalyticsProcess
    to_person_id: dict[str, str]
    to_email: dict[str, str]
    ledger: Ledger
    _test_name: str = ""
    _open: list[MetricResponse] = field(default_factory=list)

    def begin(self, test_name: str) -> None:
        self._test_name = test_name
        self._open = []

    def call(self, request: dict[str, Any]) -> MetricResponse:
        """Send `request` as the spec wrote it; the response reads in the spec's emails."""
        self.ledger.record_request(request.get("body") or {})
        status, payload = self.analytics.call_request(translate(request, self.to_person_id))
        if status != 200:
            LOG.warning("HTTP %d; body: %r", status, payload)
        response = MetricResponse(
            status, translate(payload, self.to_email), test_name=self._test_name, ledger=self.ledger
        )
        self._open.append(response)
        return response

    def end(self) -> None:
        """Every row the test selected must have its view's required fields asserted."""
        opened, self._open = self._open, []
        for response in opened:
            response.check_complete()


def run_spec(
    spec: TestYaml,
    *,
    cfg: SessionConfig,
    ch_seeder: CHSeeder,
    dbt_runner: DbtRunner,
    enrich_runner: EnrichRunner,
    analytics: AnalyticsProcess,
    identity_stub: IdentityStub,
    worker_ctx: WorkerContext,
    ledger: Ledger,
) -> SpecRun:
    """Seed, build and publish one spec; returns the caller its tests read through."""
    tracked = TrackedModels(dbt_runner, ch_seeder)
    ch_seeder.truncate_touched()
    ch_seeder.seed_bronze(spec.bronze, spec.schemas)

    staging, silver = dbt_runner.derive_selectors(spec.touched_tables)
    tracked.build(staging, worker_ctx=worker_ctx, with_ancestors=True)

    touched_schemas = {schema for schema, _ in spec.touched_tables}
    ran_enrich_steps = []
    for step in enrich_runner.steps_for(touched_schemas):
        source_ids = enrich_runner.discover_source_ids(step, spec.touched_tables)
        if not source_ids:
            continue
        for schema, table in dbt_runner.enrich_output_tables(step.name):
            ch_seeder.truncate_table(schema, table)
        enrich_runner.run(step, source_ids)
        ran_enrich_steps.append(step)

    silver_set = set(silver)
    silver_set.discard("identity_inputs")
    for step in ran_enrich_steps:
        silver_set.update(dbt_runner.ephemeral_silver_targets(step.name))
    run_only_silver = silver_set & {"class_hr_working_hours"}
    tracked.build(sorted(silver_set - run_only_silver), worker_ctx=worker_ctx)
    tracked.run(sorted(run_only_silver), worker_ctx=worker_ctx)
    if "class_collab_meeting_activity" in silver_set:
        tracked.run(["class_focus_metrics"], worker_ctx=worker_ctx, full_refresh=True)

    emails = all_persona_emails(spec.bronze)
    person_ids = person_ids_for(emails, spec.identity_aliases)
    identity_stub.allow_visible(emails)
    seed_identity_persons(cfg, person_ids)
    seed_identity_accounts(cfg, spec.identity_accounts, start_id=len(person_ids) + 1)
    dbt_runner.run("tag:identity:map", worker_ctx=worker_ctx)
    if staging or silver_set or ran_enrich_steps:
        dbt_runner.run("tag:gold", worker_ctx=worker_ctx)

    return SpecRun(
        spec=spec,
        analytics=analytics,
        to_person_id=person_ids,
        to_email=email_of_person(person_ids, spec.identity_aliases),
        ledger=ledger,
    )
