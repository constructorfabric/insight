"""What the migration would change, and every reason it refuses to.

Nothing here writes. A plan with refusals is never applied.
"""

from __future__ import annotations

from dataclasses import dataclass, field

from tenant_migration import keys
from tenant_migration.clickhouse import ClickHouse
from tenant_migration.inventory import WORK_SUFFIXES, Table, Unfit
from tenant_migration.keys import Tenants
from tenant_migration.prerequisites import jira_issues_keyed_by_issue_key, youtrack_comments_without_id

FOREIGN_SAMPLE = 5


@dataclass(frozen=True)
class TablePlan:
    table: Table
    rows: int
    moving: int
    ambiguous: int
    foreign: tuple[str, ...]
    leftovers: tuple[str, ...]


@dataclass(frozen=True)
class Plan:
    tenants: Tenants
    tables: list[TablePlan]
    refusals: list[str] = field(default_factory=list)
    comment_ids_to_fill: int = 0

    @property
    def changing(self) -> list[TablePlan]:
        return [t for t in self.tables if t.moving]


def _stats_sql(table: Table, tenants: Tenants) -> str:
    columns = table.columns
    ambiguous = keys.key_is_ambiguous(columns, tenants) if columns.key else "0"
    tenant = keys.tenant_value(columns.tenant)
    return (
        f"SELECT count(), countIf({keys.moves(columns, tenants)}), countIf({ambiguous}), "
        f"arrayStringConcat(groupUniqArrayIf({FOREIGN_SAMPLE})({tenant}, {keys.foreign_tenant(columns.tenant, tenants)}), ',') "
        f"FROM {table.sql()}"
    )


def _leftovers(ch: ClickHouse, table: Table) -> tuple[str, ...]:
    names = ", ".join(f"'{table.name}{suffix}'" for suffix in WORK_SUFFIXES)
    rows = ch.rows(f"SELECT name FROM system.tables WHERE database = '{table.database}' AND name IN ({names})")
    return tuple(f"{table.database}.{name}" for (name,) in rows)


def survey_table(ch: ClickHouse, table: Table, tenants: Tenants) -> TablePlan:
    ((rows, moving, ambiguous, foreign),) = ch.rows(_stats_sql(table, tenants))
    return TablePlan(
        table=table,
        rows=int(rows),
        moving=int(moving),
        ambiguous=int(ambiguous),
        foreign=tuple(t for t in foreign.split(",") if t),
        leftovers=_leftovers(ch, table),
    )


def refusals(tables: list[TablePlan], unfit_with_rows: list[tuple[Unfit, int]]) -> list[str]:
    """Every reason the plan cannot be applied; empty when it can."""
    reasons = [f"{u.relation}: {n} row(s), and {u.reason}" for u, n in unfit_with_rows]
    for t in tables:
        if t.foreign:
            reasons.append(
                f"{t.table.relation}: holds another tenant ({', '.join(t.foreign)}) — this install keeps several"
            )
        if t.ambiguous:
            reasons.append(
                f"{t.table.relation}: {t.ambiguous} key(s) start like a moving tenant but not with it and their source"
            )
        if t.moving and t.leftovers:
            reasons.append(
                f"{t.table.relation}: an earlier run left {', '.join(t.leftovers)} — finalize or roll back first"
            )
    return reasons


def survey(ch: ClickHouse, found: list[Table | Unfit], tenants: Tenants) -> Plan:
    tables = [survey_table(ch, t, tenants) for t in found if isinstance(t, Table)]
    unfit = [
        (u, ch.scalar(f"SELECT count() FROM `{u.relation.replace('.', '`.`', 1)}`"))
        for u in found
        if isinstance(u, Unfit)
    ]
    reasons = refusals(tables, [(u, n) for u, n in unfit if n])
    reasons += [f"{stale} — deploy this release before migrating" for stale in jira_issues_keyed_by_issue_key(ch)]
    return Plan(
        tenants=tenants, tables=tables, refusals=reasons, comment_ids_to_fill=youtrack_comments_without_id(ch, tenants)
    )
