"""The Zendesk staging models land their first rows into targets that already exist
empty, and keep landing the rows of later syncs.

A fresh install creates every staging and silver relation from the connectors-ddl
snapshot before the first sync, so the connector's first dbt run is already an
incremental one over an empty target. The models' watermark is anchored on the
target's own last build (`max(collected_at)`), and ClickHouse arithmetic on the
`max()` of an empty DateTime column wraps past 2106 — without the explicit
empty-target guard every row would fall below that boundary and the table would
stay empty forever. The second build proves the non-empty path: a row extracted
after the last build lands next to the rows already there, once.

Support has no served metric yet (#899), so this module asserts on the staging
and silver relations directly rather than through a spec.
"""

from __future__ import annotations

import json
from datetime import UTC, datetime
from typing import Any

import pytest
from insight_datapath import clickhouse, records
from insight_datapath.ch_seeder import CHSeeder
from insight_datapath.dbt_runner import DbtRunner
from insight_datapath.fixture_loader import prepare_rows
from insight_datapath.instance import InstanceConfig
from insight_datapath.records import SCHEMAS_DIR
from insight_datapath.reset import clear
from insight_datapath.tracked_models import TrackedModels
from insight_stand.manifest import Manifest

pytestmark = pytest.mark.fixture

AGENTS = "bronze_zendesk.support_agents"
EVENTS = "bronze_zendesk.support_ticket_events"
RATINGS = "bronze_zendesk.zendesk_satisfaction_ratings"

SOURCE_ID = "zendesk-datapath"
AGENT_ID = "3001"
AGENT_EMAIL = "sam.rivera@example.com"
TICKET_ID = "1001"

FIRST_AUDIT_AT = "2026-06-15T10:00:00Z"
RATING_AT = "2026-06-16T08:00:00Z"
LATER_AUDIT_AT = "2026-06-17T11:00:00Z"


def _extracted_now() -> str:
    """A sync's extract time is the present: the watermark compares it with the last build."""
    return datetime.now(tz=UTC).strftime("%Y-%m-%dT%H:%M:%SZ")


def _framing(tenant: str, key: str) -> dict[str, Any]:
    return {
        **records.framing(),
        "_airbyte_extracted_at": _extracted_now(),
        "tenant_id": tenant,
        "source_id": SOURCE_ID,
        "unique_key": f"{tenant}-{SOURCE_ID}-{key}",
        "data_source": "insight_zendesk",
        "collected_at": _extracted_now(),
    }


def _agent(tenant: str) -> dict[str, Any]:
    return {
        **_framing(tenant, AGENT_ID),
        "agent_id": AGENT_ID,
        "email": AGENT_EMAIL,
        "display_name": "Sam Rivera",
        "role": "agent",
        "is_active": 1,
    }


def _audit(
    tenant: str, audit_id: str, created_at: str, events: list[dict[str, Any]]
) -> dict[str, Any]:
    return {
        **_framing(tenant, audit_id),
        "audit_id": audit_id,
        "ticket_id": TICKET_ID,
        "author_id": AGENT_ID,
        "created_at": created_at,
        "events": json.dumps(events),
    }


def _rating(tenant: str) -> dict[str, Any]:
    return {
        **_framing(tenant, "7001"),
        "rating_id": "7001",
        "ticket_id": TICKET_ID,
        "assignee_id": AGENT_ID,
        "score": "good",
        "created_at": RATING_AT,
        "updated_at": RATING_AT,
    }


def _seed(seeder: CHSeeder, bronze: dict[str, list[dict[str, Any]]]) -> None:
    rows: dict[str, list[dict[str, Any]]] = {}
    schemas: dict[str, dict[str, Any]] = {}
    for table, raw in bronze.items():
        rows[table], schemas[table] = prepare_rows(SCHEMAS_DIR, table, raw)
    seeder.seed_bronze(rows, schemas)


def _build(tracked: TrackedModels, dbt_runner: DbtRunner, touched: set[tuple[str, str]]) -> None:
    staging, silver = dbt_runner.derive_selectors(touched)
    tracked.build(staging, with_ancestors=True)
    tracked.build(silver)


def _activity(cfg: InstanceConfig) -> list[tuple[str, int, int, int, int]]:
    """(date, public_comments, private_comments, solved, csat_good) per person-day of the source."""
    rows = clickhouse.query(
        cfg,
        "SELECT toString(date), public_comments, private_comments, solved, csat_good"
        " FROM staging.zendesk__support_activity"
        f" WHERE insight_source_id = '{SOURCE_ID}' AND person_key = '{AGENT_EMAIL}'"
        " ORDER BY date",
    )
    return [
        (str(d), int(pub), int(prv), int(solved), int(good)) for d, pub, prv, solved, good in rows
    ]


def _count(cfg: InstanceConfig, relation: str) -> int:
    rows = clickhouse.query(
        cfg, f"SELECT count() FROM {relation} WHERE insight_source_id = '{SOURCE_ID}'"
    )
    return int(rows[0][0])


def test_first_build_lands_into_empty_targets_and_a_later_sync_lands_on_top(
    ch_seeder: CHSeeder,
    dbt_runner: DbtRunner,
    instance_cfg: InstanceConfig,
    stand_manifest: Manifest,
) -> None:
    tenant = stand_manifest.tenant
    tracked = TrackedModels(dbt_runner, ch_seeder)
    clear(ch_seeder.cfg, ch_seeder.ledger.drain())
    touched = {
        (schema, table)
        for schema, table in (name.split(".", 1) for name in (AGENTS, EVENTS, RATINGS))
    }

    first_sync = {
        AGENTS: [_agent(tenant)],
        EVENTS: [
            _audit(
                tenant,
                "9001",
                FIRST_AUDIT_AT,
                [
                    {"id": 90011, "type": "Comment", "public": True},
                    {"id": 90012, "type": "Change", "field_name": "status", "value": "solved"},
                ],
            )
        ],
        RATINGS: [_rating(tenant)],
    }
    _seed(ch_seeder, first_sync)
    _build(tracked, dbt_runner, touched)

    assert _count(instance_cfg, "staging.zendesk__support_event") == 2, (
        "a public comment and a solve"
    )
    assert _activity(instance_cfg) == [
        ("2026-06-15", 1, 0, 1, 0),
        ("2026-06-16", 0, 0, 0, 1),
    ], "the first build over an empty target must be a full build"
    assert _count(instance_cfg, "silver.class_support_activity") == 2

    later_sync = {
        EVENTS: [
            _audit(
                tenant, "9002", LATER_AUDIT_AT, [{"id": 90021, "type": "Comment", "public": False}]
            )
        ],
    }
    _seed(ch_seeder, later_sync)
    _build(tracked, dbt_runner, touched)

    assert _count(instance_cfg, "staging.zendesk__support_event") == 3, (
        "the later audit lands next to the first"
    )
    assert _activity(instance_cfg) == [
        ("2026-06-15", 1, 0, 1, 0),
        ("2026-06-16", 0, 0, 0, 1),
        ("2026-06-17", 0, 1, 0, 0),
    ], "a non-empty target keeps its rows and gains the new day, once"
    assert _count(instance_cfg, "silver.class_support_activity") == 3
