"""Writing generated rows into the warehouse.

The generators hardcode their column lists; the schema they write into is built
by `create-bronze-placeholders.sh` from the connectors-ddl snapshot and healed by
the migrations. The two drift, so what a generator wrote is reconciled against
what the target can hold before anything is sent — `plan_insert` decides that
over values, `bulk_insert` is the shell that reads the shape and sends the rows.
"""

from __future__ import annotations

import datetime as _dt
import hashlib
import logging
import re
from dataclasses import dataclass
from typing import TYPE_CHECKING

from .base import UTC, deterministic_uuid

LOG = logging.getLogger("seed.generators")

if TYPE_CHECKING:
    import clickhouse_connect.driver.client


#: Every relation the generators clear before writing — the seed's destructive
#: surface, in one place because `preflight` has to refuse a stand whose data
#: sits in exactly these and nowhere else. `truncate` rejects an unregistered
#: target, and `test_preflight.py` scans the call sites to keep the two in step:
#: a new generator that clears a table nobody registered fails the test rather
#: than quietly widening what a seed run destroys.
RESET_TARGETS: tuple[tuple[str, str], ...] = (
    ("bronze_bamboohr", "employees"),
    ("bronze_bitbucket_cloud", "repositories"),
    ("bronze_claude_team_invoices", "claude_team_invoice_lines"),
    ("bronze_github", "deployment_statuses"),
    ("bronze_github", "deployments"),
    ("bronze_github", "repositories"),
    ("bronze_github", "workflow_runs"),
    ("bronze_gitlab", "projects"),
    ("silver", "class_ai_assistant_usage"),
    ("silver", "class_ai_dev_usage"),
    ("silver", "class_ai_invoice"),
    ("silver", "class_ai_overage"),
    ("silver", "class_collab_chat_activity"),
    ("silver", "class_collab_email_activity"),
    ("silver", "class_collab_meeting_activity"),
    ("silver", "class_crm_activities"),
    ("silver", "class_crm_deals"),
    ("silver", "class_crm_users"),
    ("silver", "class_focus_metrics"),
    ("silver", "class_git_ci_runs"),
    ("silver", "class_git_commits"),
    ("silver", "class_git_deployment_events"),
    ("silver", "class_git_deployments"),
    ("silver", "class_git_file_changes"),
    ("silver", "class_git_pull_requests"),
    ("silver", "class_git_pull_requests_commits"),
    ("silver", "class_git_repositories"),
    ("silver", "class_people"),
    ("silver", "class_support_activity"),
    ("silver", "class_task_field_history"),
    ("silver", "class_task_issuetypes"),
    ("silver", "class_task_statuses"),
    ("silver", "class_task_users"),
    ("silver", "class_task_worklogs"),
    ("silver", "class_wiki_activity"),
    ("silver", "class_wiki_engagement"),
    ("silver", "class_wiki_pages"),
    ("staging", "bitbucket_cloud__repositories"),
    ("staging", "claude_team__ai_invoice"),
    ("staging", "github__ci_runs"),
    ("staging", "github__deployment_events"),
    ("staging", "github__deployments"),
    ("staging", "github__repositories"),
    ("staging", "gitlab__repositories"),
)


def truncate(
    client: clickhouse_connect.driver.client.Client,
    schema: str,
    table: str,
) -> None:
    """Idempotent reset: TRUNCATE before INSERT."""
    if (schema, table) not in RESET_TARGETS:
        raise ValueError(
            f"{schema}.{table} is not in RESET_TARGETS. Clearing a relation preflight "
            "does not know about would let a seed run destroy data it never warned about "
            "— register it there (and in the same commit) instead."
        )
    client.command(f"TRUNCATE TABLE IF EXISTS `{schema}`.`{table}`")


@dataclass(frozen=True)
class TargetShape:
    """What the warehouse says about an insert target.

    `columns` maps name to type and holds only what a generator may write:
    MATERIALIZED and ALIAS columns are left out, because ClickHouse computes
    them and refuses an explicit value — so a generator naming one is a column
    to drop, and the sorting-key guard must not demand one.

    `key_columns` is what the server counts towards the sorting key, not what
    the key string spells: `ORDER BY (person_id, toDate(ts))` marks `ts` as much
    as `person_id`, and a defaulted `ts` makes `toDate(ts)` constant just as
    surely. `sorting_key` keeps the verbatim expression, for saying so.
    """

    columns: dict[str, str]
    key_columns: tuple[str, ...]
    engine: str
    sorting_key: str


@dataclass(frozen=True)
class InsertPlan:
    columns: tuple[str, ...]
    rows: tuple[tuple[object, ...], ...]
    dropped: tuple[str, ...]
    key_synthesised: bool
    defaulted: tuple[str, ...]


#: Engine families that keep one row per sorting key. On any other engine a
#: duplicated sorting key is untidy; on these it is row loss.
_COLLAPSING_ENGINES = ("Replacing", "Collapsing", "Summing", "Aggregating")


def collapsing_key_columns(shape: TargetShape) -> list[str]:
    """Columns whose value decides which rows survive a merge.

    Empty for an engine that keeps every row.
    """
    if not any(family in shape.engine for family in _COLLAPSING_ENGINES):
        return []
    return list(shape.key_columns)


def _target_shape(
    client: clickhouse_connect.driver.client.Client,
    schema: str,
    table: str,
) -> TargetShape:
    """Read the target's shape. The only part of an insert that needs a server."""
    columns = client.query(
        "SELECT name, type, is_in_sorting_key FROM system.columns "
        "WHERE database = {db:String} AND table = {tbl:String} "
        "AND default_kind NOT IN ('MATERIALIZED', 'ALIAS')",
        parameters={"db": schema, "tbl": table},
    )
    table_row = client.query(
        "SELECT engine, sorting_key FROM system.tables "
        "WHERE database = {db:String} AND name = {tbl:String}",
        parameters={"db": schema, "tbl": table},
    )
    engine, sorting_key = (
        (str(table_row.result_rows[0][0]), str(table_row.result_rows[0][1]))
        if table_row.result_rows
        else ("", "")
    )
    return TargetShape(
        columns={row[0]: row[1] for row in columns.result_rows},
        key_columns=tuple(row[0] for row in columns.result_rows if row[2]),
        engine=engine,
        sorting_key=sorting_key,
    )


def _coerce(value: object, ch_type: str) -> object:
    """Widen a `date` to midnight UTC `datetime` for DateTime-typed columns.

    The snapshot schema types some day-grain columns as `DateTime`/
    `DateTime64` where a generator supplies `datetime.date`; the driver then
    calls `.timestamp()` on it and raises. A date means midnight on that
    date, so the widening is lossless and unambiguous. Anything else is
    passed through untouched — this is not a general type converter.

    The result MUST be timezone-aware. clickhouse-connect serialises
    DateTime/DateTime64 with `int(x.timestamp())`, and `.timestamp()` on a
    NAIVE datetime resolves it in the seeding *process's* local timezone. A
    naive midnight therefore lands 8h early on a UTC+8 host — i.e. on the
    previous calendar day once ClickHouse renders it in UTC — silently
    misdating every affected row. That is invisible in the seed-sample
    container (UTC) but real for the host-side run CONTRIBUTING.md
    documents. `.timestamp()` on an AWARE datetime is host-independent.
    """
    if (
        isinstance(value, _dt.date)
        and not isinstance(value, _dt.datetime)
        and "DateTime" in ch_type
    ):
        return _dt.datetime.combine(value, _dt.time(), tzinfo=UTC)
    return value


def _synthesised_keys(
    schema: str,
    table: str,
    columns: list[str],
    rows: list[tuple[object, ...]],
    declared_type: str,
) -> list[str]:
    """A `unique_key` per row, derived from the row's own values.

    INVARIANT: no `_`-prefixed column feeds the key. `_version` moving the key
    would append a newer row beside the old one instead of replacing it, which
    inverts ReplacingMergeTree.
    """
    # Respect the column's declared width: unique_key is FixedString(N) on some
    # tables and String on others.
    width_match = re.search(r"FixedString\((\d+)\)", declared_type)
    width = int(width_match.group(1)) if width_match else None
    key_idx = [i for i, c in enumerate(columns) if not c.startswith("_")]

    keys = []
    for row in rows:
        seed = "|".join(repr(row[i]) for i in key_idx)
        digest = hashlib.blake2b(f"{schema}|{table}|{seed}".encode(), digest_size=16).hexdigest()
        keys.append(digest[:width] if width else deterministic_uuid(schema, table, seed))
    return keys


def plan_insert(
    schema: str,
    table: str,
    columns: list[str],
    rows: list[tuple[object, ...]],
    shape: TargetShape,
) -> InsertPlan:
    """Reconcile what a generator wrote against what the target can hold.

    The live schema is authoritative in both directions, and they are not
    symmetric: a column the target cannot hold is dropped, so a generator's list
    may lag the snapshot without aborting a seed, while a column the target has
    and the generator omits takes the engine default — refused when a collapsing
    engine's sorting key reads it, since every row would default to the same
    value and the table would keep one of them at the next merge.

    Raises `RuntimeError` when the target holds no writable column at all (it
    does not exist, or the caller is pointed at the wrong database), and when a
    collapsing key column is left to the default.
    """
    if not shape.columns:
        raise RuntimeError(
            f"{schema}.{table} has no columns (does the table exist?) — "
            f"cannot reconcile generator columns {columns}"
        )

    dropped = tuple(c for c in columns if c not in shape.columns)
    if dropped:
        keep = [i for i, c in enumerate(columns) if c in shape.columns]
        columns = [columns[i] for i in keep]
        rows = [tuple(row[i] for i in keep) for row in rows]

    key_synthesised = "unique_key" in shape.columns and "unique_key" not in columns
    if key_synthesised:
        keys = _synthesised_keys(schema, table, columns, rows, shape.columns["unique_key"])
        columns = [*columns, "unique_key"]
        rows = [(*row, key) for row, key in zip(rows, keys, strict=True)]

    key_columns = collapsing_key_columns(shape)
    omitted_key = [c for c in key_columns if c not in columns]
    if omitted_key:
        raise RuntimeError(
            f"{schema}.{table} is a {shape.engine}, which keeps ONE row per sorting key "
            f"({shape.sorting_key}), and this generator writes no value for "
            f"{', '.join(omitted_key)}. Every row would carry the engine default there, "
            "so the key stops telling the rows apart and the table collapses on merge — "
            "invisible until it has. Write the column, or drop it from the sorting key."
        )

    types = [shape.columns[c] for c in columns]
    if any("DateTime" in t for t in types):
        rows = [tuple(_coerce(v, t) for v, t in zip(row, types, strict=True)) for row in rows]

    return InsertPlan(
        columns=tuple(columns),
        rows=tuple(rows),
        dropped=dropped,
        key_synthesised=key_synthesised,
        defaulted=tuple(c for c in shape.columns if c not in columns),
    )


def bulk_insert(
    client: clickhouse_connect.driver.client.Client,
    schema: str,
    table: str,
    columns: list[str],
    rows: list[tuple[object, ...]],
) -> int:
    """Insert `rows` and return the count. No-op on empty input.

    The shell around `plan_insert`: read the target's shape, say out loud what
    reconciling against it changed, send the rows.
    """
    if not rows:
        return 0

    plan = plan_insert(schema, table, columns, rows, _target_shape(client, schema, table))

    if plan.dropped:
        LOG.warning(
            "%s.%s: dropping %d generator column(s) absent from the live schema: %s",
            schema,
            table,
            len(plan.dropped),
            ", ".join(plan.dropped),
        )
    if plan.key_synthesised:
        LOG.warning(
            "%s.%s: generator omits unique_key on a ReplacingMergeTree table; "
            "synthesising it per row to prevent dedup collapse",
            schema,
            table,
        )
    if plan.defaulted:
        LOG.info(
            "%s.%s: %d column(s) left to the engine default: %s",
            schema,
            table,
            len(plan.defaulted),
            ", ".join(plan.defaulted),
        )

    client.insert(table, list(plan.rows), column_names=list(plan.columns), database=schema)
    return len(plan.rows)
