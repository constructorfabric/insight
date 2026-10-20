from __future__ import annotations

import re

TOPOLOGIES = ("standalone", "replicated")


def replicated_statements(sql: str) -> tuple[str, ...]:
    database = re.fullmatch(r"CREATE DATABASE (silver|insight|benchmark)", sql.strip())
    if database:
        name = database[1]
        return (
            (
                f"CREATE DATABASE {name} ON CLUSTER rig "
                f"ENGINE = Replicated('/clickhouse/databases/{name}', '{{shard}}', '{{replica}}')"
            ),
        )
    if not sql.lstrip().startswith("CREATE TABLE "):
        return (sql,)
    sql = re.sub(r"\bENGINE\s*=\s*(MergeTree|ReplacingMergeTree)\b", r"ENGINE = Replicated\1", sql)
    create = re.fullmatch(r"(CREATE TABLE (\w+\.\w+)\b[\s\S]*?)\s+AS\s+([\s\S]+)", sql.strip())
    if create:
        return (f"{create[1]} EMPTY AS {create[3]}", f"INSERT INTO {create[2]} {create[3]}")
    return (sql,)


def changed_database(sql: str) -> str | None:
    match = re.match(r"(?:CREATE|DROP|TRUNCATE) TABLE (?:IF EXISTS )?(\w+)\.", sql.strip())
    return match[1] if match else None
