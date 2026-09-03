from __future__ import annotations

import logging
from typing import Any

import pytest
from lib.analytics import AnalyticsProcess
from lib.ch_seeder import CHSeeder
from lib.dbt_runner import DbtRunner
from lib.enrich import EnrichRunner
from lib.expect_engine import evaluate_case
from lib.fixture_loader import TestYaml
from lib.identity_stub import IdentityStub
from lib.spec_runner import _EMAIL_TOKEN, person_ids_for, seed_identity_accounts, seed_identity_persons, translate
from lib.tracked_models import TrackedModels
from lib.worker import WorkerContext

pytestmark = pytest.mark.fixture
LOG = logging.getLogger("e2e.runner")


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


# The reserved not-a-human person (bots, CI): an account bound to it claims
# nothing in either resolution map. Mirrors excluded_person_id() in dbt.


# One synthetic connector instance for every rig persona: the account triple is
# what joins evidence to bindings, so both inserts must name the same source.


def test_metric_smoke(
    test_yaml: TestYaml,
    ch_seeder: CHSeeder,
    dbt_runner: DbtRunner,
    enrich_runner: EnrichRunner,
    analytics: AnalyticsProcess,
    identity_stub: IdentityStub,
    tracked_models: TrackedModels,
    worker_ctx: WorkerContext,
) -> None:
    ch_seeder.truncate_touched()

    # 2. Seed this test's resolved bronze records.
    ch_seeder.seed_bronze(test_yaml.bronze, test_yaml.schemas)

    # 3. Build the dbt models the seeded tables feed: staging first (the `+`
    #    pulls <connector>__bronze_promoted), then the silver class models.
    staging, silver = dbt_runner.derive_selectors(test_yaml.touched_tables)
    tracked_models.build(staging, worker_ctx=worker_ctx, with_ancestors=True)
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
    tracked_models.build(sorted(tested_silver), worker_ctx=worker_ctx)
    tracked_models.run(sorted(run_only_silver), worker_ctx=worker_ctx)
    if "class_collab_meeting_activity" in silver_set:
        tracked_models.run(["class_focus_metrics"], worker_ctx=worker_ctx, full_refresh=True)

    # 4. Identity bindings for the personas the cases address (the rig plays the
    #    persons-sync role). Must precede the gold build: the cohorts view
    #    INNER JOINs person_map, and creating a view validates its references.
    persona_emails = _all_persona_emails(test_yaml)
    all_person_ids = person_ids_for(persona_emails, test_yaml.identity_aliases)
    to_person_id = {email: all_person_ids[email] for email in _requested_persona_emails(test_yaml)}
    # The visibility gate asks the stub about the ids this case requests, so
    # the stub's visible set is derived from the yaml — never a hand-kept list
    # a new persona could fall outside of (that reads as an authz bug).
    identity_stub.allow_visible(persona_emails)
    seed_identity_persons(ch_seeder.cfg, all_person_ids)
    seed_identity_accounts(ch_seeder.cfg, test_yaml.identity_accounts, start_id=len(all_person_ids) + 1)

    # Selected without upstream: `identity_inputs` is hand-created above and
    # excluded from the silver selection.
    dbt_runner.run("tag:identity:map", worker_ctx=worker_ctx)

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
        status, payload = analytics.call_request(translate(case["request"], to_person_id))
        if status != 200:
            LOG.warning("HTTP %d; body: %r", status, payload)
        evaluate_case(case, translate(payload, to_email), status)
