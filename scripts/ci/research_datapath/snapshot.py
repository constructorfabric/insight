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


def serving_nodes(manifest: dict[str, object]) -> list[dict[str, object]]:
    models = {key: node for key, node in manifest["nodes"].items() if node["resource_type"] == "model"}
    by_name = {node["name"]: key for key, node in models.items()}
    ordered = []
    seen = set()

    def visit(key: str) -> None:
        if key in seen:
            return
        seen.add(key)
        node = models[key]
        if node["config"]["materialized"] == "view":
            for dependency in node["depends_on"]["nodes"]:
                if dependency in models:
                    visit(dependency)
        ordered.append(node)

    for key, node in models.items():
        if "gold" in node["tags"] and node["config"]["materialized"] in ("table", "incremental"):
            visit(key)
    visit(by_name["metric_entity_cohorts_current"])
    return ordered


def capture(cfg: InstanceConfig, tenant: str, target: Path, output: Path) -> None:
    manifest = json.loads((target / "manifest.json").read_text())
    output.mkdir()
    tables = []
    names = set()
    views = []
    nodes = serving_nodes(manifest)
    selected = {node["unique_id"] for node in nodes}
    omitted = [
        {"name": node["name"], "materialized": node["config"]["materialized"]}
        for node in manifest["nodes"].values()
        if node["resource_type"] == "model" and "gold" in node["tags"] and node["unique_id"] not in selected
    ]
    nodes.insert(
        0, {"schema": "identity", "alias": "identity_persons", "config": {"materialized": "table"}, "stand_owned": True}
    )
    for node in nodes:
        materialized = node["config"]["materialized"]
        if materialized not in ("table", "incremental", "view"):
            raise ValueError("unsupported serving materialization")
        database, table = node["schema"], node["alias"]
        if (database, table) in names:
            raise ValueError("duplicate gold relation")
        names.add((database, table))
        relation = f"{identifier(database)}.{identifier(table)}"
        with ch.client(cfg) as client:
            ddl = client.query(f"SHOW CREATE TABLE {relation}").result_rows[0][0]
            if materialized == "view":
                views.append({"database": database, "table": table, "ddl": ddl})
                continue
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
                "stand_owned": bool(node.get("stand_owned")),
            }
        )
    if not tables:
        raise ValueError("no gold tables captured")
    (output / "snapshot.json").write_text(
        json.dumps(
            {
                "tenant": tenant,
                "tables": tables,
                "views": views,
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
            exists = int(client.command(f"EXISTS TABLE {relation}"))
            if not exists:
                client.command(entry["ddl"])
            elif not entry["stand_owned"]:
                raise ValueError(f"fixture target already exists: {relation}")
            if client.query(f"SHOW CREATE TABLE {relation}").result_rows[0][0] != entry["ddl"]:
                raise AssertionError(f"DDL changed for {relation}")
            native = (snapshot / entry["file"]).read_bytes()
            if hashlib.sha256(native).hexdigest() != entry["sha256"]:
                raise ValueError("gold fixture bytes changed")
            if exists and entry["stand_owned"]:
                continue
            if entry["rows"]:
                client.raw_insert(relation, column_names=entry["columns"], insert_block=native, fmt="Native")
            if int(client.command(f"SELECT count() FROM {relation}")) != entry["rows"]:
                raise AssertionError(f"row count differs for {relation}")
        for entry in document["views"]:
            relation = f"{identifier(entry['database'])}.{identifier(entry['table'])}"
            client.command(entry["ddl"])
            if client.query(f"SHOW CREATE TABLE {relation}").result_rows[0][0] != entry["ddl"]:
                raise AssertionError(f"view DDL changed for {relation}")
    duration_values(cfg, tenant)
