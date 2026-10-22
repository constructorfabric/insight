"""Repairs the key rewrite depends on, checked or applied before any table moves.

Both read a row's identity out of its key, so they must see the key it was
written with.
"""

from __future__ import annotations

from tenant_migration import ddl
from tenant_migration.clickhouse import ClickHouse
from tenant_migration.keys import Tenants

JIRA_ISSUE_TABLES = ("jira_issue", "jira_issue_keys")
YOUTRACK_COMMENTS = "`bronze_youtrack`.`youtrack_comments`"


def _exists(ch: ClickHouse, database: str, table: str, column: str | None = None) -> bool:
    where = f"database = '{database}' AND table = '{table}'" + (f" AND name = '{column}'" if column else "")
    return ch.scalar(f"SELECT count() FROM system.columns WHERE {where}") > 0


def jira_issues_keyed_by_issue_key(ch: ClickHouse) -> list[str]:
    """Jira issue tables still keyed by the issue key: the deploy hook's heal has not run."""
    pending = []
    for table in JIRA_ISSUE_TABLES:
        if not _exists(ch, "bronze_jira", table, "jira_id"):
            continue
        stale = ch.scalar(
            f"SELECT count() FROM `bronze_jira`.`{table}` WHERE coalesce(unique_key, '') != "
            "concat(coalesce(tenant_id, ''), '-', coalesce(source_id, ''), '-', coalesce(jira_id, ''))"
        )
        if stale:
            pending.append(f"bronze_jira.{table}: {stale} row(s) keyed by the issue key")
    return pending


def _comment_id_from_key(tenants: Tenants) -> tuple[str, str]:
    """(the key starts with a known tenant and the row's source, the comment id after them)."""
    conditions, ids = [], []
    for tenant in (*tenants.moving, tenants.new):
        prefix = f"concat({ddl.literal(tenant)}, '-', ifNull(source_id, ''), '-')"
        conditions.append(f"startsWith(unique_key, {prefix})")
        ids += [conditions[-1], f"substring(unique_key, length({prefix}) + 1)"]
    return " OR ".join(conditions), f"multiIf({', '.join(ids)}, NULL)"


def youtrack_comments_without_id(ch: ClickHouse, tenants: Tenants) -> int:
    if not _exists(ch, "bronze_youtrack", "youtrack_comments", "id"):
        return 0
    known, _ = _comment_id_from_key(tenants)
    return ch.scalar(f"SELECT count() FROM {YOUTRACK_COMMENTS} WHERE id IS NULL AND ({known})")


def fill_youtrack_comment_ids(ch: ClickHouse, tenants: Tenants) -> None:
    """Old comment rows know their id only from the key; write it down before the key changes."""
    if not youtrack_comments_without_id(ch, tenants):
        return
    known, comment_id = _comment_id_from_key(tenants)
    ch.execute(
        f"ALTER TABLE {YOUTRACK_COMMENTS} UPDATE id = {comment_id} "
        f"WHERE id IS NULL AND ({known}) SETTINGS mutations_sync = 2"
    )
