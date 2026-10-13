"""Every statement the migration sends that names a table, built in one place.

ClickHouse takes no parameters for identifiers, so names are interpolated.
SAFETY: a `Relation` admits only plain identifiers (see inventory.IDENTIFIER),
every name is backquoted, and the only literal — a table comment — is escaped.
"""

from __future__ import annotations

from tenant_migration.inventory import Relation


def show_create(rel: Relation) -> str:
    return f"SHOW CREATE TABLE {rel.sql()}"


def create_like(rel: Relation, suffix: str) -> str:
    return f"CREATE TABLE {rel.sql(suffix)} AS {rel.sql()}"


def insert_rewritten(rel: Relation, suffix: str, replacements: str) -> str:
    return f"INSERT INTO {rel.sql(suffix)} SELECT * REPLACE ({replacements}) FROM {rel.sql()}"


def drop(rel: Relation, suffix: str, *, if_exists: bool = True) -> str:
    return f"DROP TABLE {'IF EXISTS ' if if_exists else ''}{rel.sql(suffix)}"


def exchange(rel: Relation, a: str, b: str) -> str:
    return f"EXCHANGE TABLES {rel.sql(a)} AND {rel.sql(b)}"


def rename(rel: Relation, source: str, target: str) -> str:
    return f"RENAME TABLE {rel.sql(source)} TO {rel.sql(target)}"


def set_comment(rel: Relation, suffix: str, comment: str) -> str:
    escaped = comment.replace("\\", "\\\\").replace("'", "\\'")
    return f"ALTER TABLE {rel.sql(suffix)} MODIFY COMMENT '{escaped}'"
