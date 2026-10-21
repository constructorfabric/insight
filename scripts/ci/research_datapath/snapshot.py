from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path

from insight_datapath import clickhouse as ch
from insight_datapath.instance import InstanceConfig


def identifier(value: str) -> str:
    if not value or not all(c.isalnum() or c == "_" for c in value):
        raise ValueError("invalid fixture relation name")
    return f"`{value}`"


def duration_values(cfg: InstanceConfig, tenant: str) -> None:
    rows = ch.query(
        cfg,
        f"SELECT value FROM insight.ci_metric_observations WHERE tenant_id = {ch.literal(tenant)} AND measure_key = 'run_duration_min' ORDER BY value",
    )
    if [row[0] for row in rows] != [1.0, 2.0, 10.0]:
        raise AssertionError(f"gold durations differ from independent fixture expectations: {rows}")


def capture(cfg: InstanceConfig, tenant: str, target: Path, output: Path) -> None:
    manifest = json.loads((target / "manifest.json").read_text())
    output.mkdir()
    tables = []
    names = set()
    omitted = []
    for node in manifest["nodes"].values():
        if node["resource_type"] != "model" or "gold" not in node["tags"]:
            continue
        materialized = node["config"]["materialized"]
        if materialized in ("ephemeral", "view"):
            omitted.append({"name": node["name"], "materialized": materialized})
            continue
        if materialized not in ("table", "incremental"):
            raise ValueError("unsupported gold materialization")
        database, table = node["schema"], node["alias"]
        if (database, table) in names:
            raise ValueError("duplicate gold relation")
        names.add((database, table))
        relation = f"{identifier(database)}.{identifier(table)}"
        with ch.client(cfg) as client:
            ddl = client.query(f"SHOW CREATE TABLE {relation}").result_rows[0][0]
            columns = [
                row[0]
                for row in client.query(
                    f"SELECT name FROM system.columns WHERE database={ch.literal(database)} AND table={ch.literal(table)} ORDER BY position"
                ).result_rows
            ]
            count = client.command(f"SELECT count() FROM {relation}")
            selected = ", ".join(identifier(column) for column in columns)
            native = client.raw_query(f"SELECT {selected} FROM {relation}", fmt="Native")
        filename = f"{database}.{table}.native"
        (output / filename).write_bytes(native)
        tables.append(
            {
                "database": database,
                "table": table,
                "ddl": ddl,
                "columns": columns,
                "rows": int(count),
                "file": filename,
                "sha256": hashlib.sha256(native).hexdigest(),
            }
        )
    if not tables:
        raise ValueError("no gold tables captured")
    (output / "snapshot.json").write_text(
        json.dumps(
            {
                "tenant": tenant,
                "tables": tables,
                "omitted_non_table_models": omitted,
                "source_sha": os.environ["INSIGHT_RESEARCH_SOURCE_SHA"],
            },
            indent=2,
        )
        + "\n"
    )


def replay(cfg: InstanceConfig, tenant: str, snapshot: Path) -> None:
    document = json.loads((snapshot / "snapshot.json").read_text())
    if document["source_sha"] != os.environ["INSIGHT_RESEARCH_SOURCE_SHA"]:
        raise ValueError("gold fixture belongs to another source revision")
    if document["tenant"] != tenant:
        raise ValueError("snapshot tenant differs from the isolated stand")
    with ch.client(cfg) as client:
        for entry in document["tables"]:
            database, table = entry["database"], entry["table"]
            relation = f"{identifier(database)}.{identifier(table)}"
            client.command(f"CREATE DATABASE IF NOT EXISTS {identifier(database)}")
            client.command(entry["ddl"])
            if client.query(f"SHOW CREATE TABLE {relation}").result_rows[0][0] != entry["ddl"]:
                raise AssertionError(f"DDL changed for {relation}")
            native = (snapshot / entry["file"]).read_bytes()
            if hashlib.sha256(native).hexdigest() != entry["sha256"]:
                raise ValueError("gold fixture bytes changed")
            if entry["rows"]:
                client.raw_insert(relation, column_names=entry["columns"], insert_block=native, fmt="Native")
            if int(client.command(f"SELECT count() FROM {relation}")) != entry["rows"]:
                raise AssertionError(f"row count differs for {relation}")
    duration_values(cfg, tenant)
