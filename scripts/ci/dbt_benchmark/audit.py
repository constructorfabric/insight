from __future__ import annotations

from client import Client


def check_topology(client: Client, topology: str) -> dict[str, object]:
    snapshots = []
    for node in client.nodes():
        databases = node.rows(
            "SELECT name, engine FROM system.databases WHERE name IN ('silver', 'insight', 'benchmark') ORDER BY name"
        )
        tables = node.rows(
            "SELECT database, name, engine FROM system.tables WHERE database IN ('silver', 'insight', 'benchmark') "
            "ORDER BY database, name"
        )
        if len(databases) != 3 or not tables:
            raise AssertionError("benchmark databases or tables are missing on a node")
        replicated = topology == "replicated"
        if any((database["engine"] == "Replicated") != replicated for database in databases):
            raise AssertionError("database engine does not match the benchmark topology")
        for table in tables:
            engine = table["engine"]
            if "MergeTree" in engine and engine.startswith("Replicated") != replicated:
                raise AssertionError("table engine does not match the benchmark topology")
            if "__dbt_" in table["name"]:
                raise AssertionError("a dbt intermediate was left in the benchmark warehouse")
        snapshots.append((databases, tables))
    if any(snapshot != snapshots[0] for snapshot in snapshots):
        raise AssertionError("benchmark metadata differs between nodes")
    return {"node_count": len(snapshots), "engines_match": True, "metadata_equal": True}
