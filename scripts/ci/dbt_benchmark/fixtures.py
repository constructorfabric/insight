from __future__ import annotations

import json
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

from client import Client
from experiments import MODEL, SERVING

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
DBT = ROOT / "src/ingestion/dbt"

READ_PROBES = {
    "month_measure": "tenant_id = 'synthetic' AND source_key = 'synthetic_source' "
    "AND entity_type = 'person' AND measure_key = 'measure-2' "
    "AND metric_date >= toDate('2024-06-01') AND metric_date < toDate('2024-07-01')",
    "month_entity": "tenant_id = 'synthetic' AND source_key = 'synthetic_source' "
    "AND entity_type = 'person' AND entity_id = '42' "
    "AND measure_key = 'measure-2' AND metric_date >= toDate('2024-06-01') "
    "AND metric_date < toDate('2024-07-01')",
    "year_measure": "tenant_id = 'synthetic' AND source_key = 'synthetic_source' "
    "AND entity_type = 'person' AND measure_key = 'measure-2' "
    "AND metric_date >= toDate('2024-01-01') AND metric_date < toDate('2025-01-01')",
}


@dataclass(frozen=True)
class Workload:
    name: str
    sql: str
    settings: dict[str, str | int]
    order_by: str
    partition_by: str


def run_dbt(profiles: Path, target: Path) -> dict[str, object]:
    subprocess.run(
        [
            sys.executable,
            str(ROOT / "src/ingestion/scripts/dbt_profiles.py"),
            "--target",
            "benchmark",
            "--profiles-dir",
            str(profiles),
        ],
        check=True,
    )
    subprocess.run(
        [
            str(Path(sys.executable).parent / "dbt"),
            "compile",
            "--select",
            MODEL,
            "--profiles-dir",
            str(profiles),
            "--target-path",
            str(target),
            "--log-path",
            str(target / "logs"),
        ],
        cwd=DBT,
        check=True,
    )
    return json.loads((target / "manifest.json").read_text())["nodes"]


def query_settings(settings: dict[str, object]) -> dict[str, str | int]:
    parsed = {}
    for key, value in settings.items():
        if not isinstance(value, (str, int)):
            raise TypeError(f"unsupported query setting {key}: {value!r}")
        parsed[key] = value
    return parsed


def prepare(client: Client, rows: int, work: Path) -> dict[str, Workload]:
    existing = client.rows(
        "SELECT database, name FROM system.tables WHERE database NOT IN "
        "('system', 'INFORMATION_SCHEMA', 'information_schema')"
    )
    if existing:
        raise RuntimeError("benchmark requires a fresh disposable ClickHouse; refusing an existing warehouse")
    for sql in (HERE / "seed.sql").read_text().split(";"):
        if sql.strip():
            client.execute(sql, settings={"param_rows": rows})
    profiles = work / "profiles"
    profiles.mkdir()
    nodes = run_dbt(profiles, work / "target")
    model = nodes[f"model.ingestion.{MODEL}"]
    serving = nodes["model.ingestion.git_metric_evidence"]["config"]
    settings = query_settings(model["config"]["query_settings"])
    workloads = {
        MODEL: Workload(
            MODEL,
            model["compiled_code"].strip().rstrip(";"),
            settings,
            ", ".join(model["config"]["order_by"]),
            "tuple()",
        ),
        SERVING: Workload(
            SERVING,
            (HERE / "serving.sql").read_text().strip(),
            query_settings(serving["query_settings"]),
            ", ".join(serving["order_by"]),
            serving["partition_by"],
        ),
    }
    queries = {
        "aggregation": "SELECT group_id, max(payload) AS payload FROM benchmark.input GROUP BY group_id",
        "sort": "SELECT id, payload FROM benchmark.input ORDER BY payload, id",
        "join": "SELECT l.id, r.payload FROM benchmark.input AS l INNER JOIN benchmark.input AS r ON l.id = r.id",
    }
    for name, sql in queries.items():
        workloads[name] = Workload(name, sql, settings, "tuple()", "tuple()")
    return workloads


def targets(client: Client, workload: Workload, partition: str) -> None:
    for name in ("expected", "baseline", "candidate"):
        client.execute("DROP TABLE IF EXISTS benchmark.{table:Identifier}", settings={"param_table": name})
    # SAFETY: all fragments are repo-owned SQL or compiled model configuration, not external data values.
    # nosemgrep: python.sqlalchemy.security.sqlalchemy-execute-raw-query.sqlalchemy-execute-raw-query
    client.execute(
        f"CREATE TABLE benchmark.expected ENGINE = MergeTree PARTITION BY ({workload.partition_by}) "
        f"ORDER BY ({workload.order_by}) AS {workload.sql}",
        settings=workload.settings,
    )
    for name, expression in (("baseline", workload.partition_by), ("candidate", partition)):
        # SAFETY: table names are fixed and expressions come from the compiled model or experiment registry.
        # nosemgrep: python.sqlalchemy.security.sqlalchemy-execute-raw-query.sqlalchemy-execute-raw-query
        client.execute(
            f"CREATE TABLE benchmark.{name} ENGINE = MergeTree PARTITION BY ({expression}) "
            f"ORDER BY ({workload.order_by}) AS SELECT * FROM benchmark.expected WHERE 0"
        )
