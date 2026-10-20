from __future__ import annotations

import argparse
import json
import os
import platform
import random
import subprocess
import sys
from pathlib import Path
from statistics import median
from tempfile import TemporaryDirectory
from time import perf_counter, sleep
from uuid import uuid4

from client import Client

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
DBT = ROOT / "src/ingestion/dbt"
MODEL = "git_authored_commits"
MIB = 1024 * 1024


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
    manifest = json.loads((target / "manifest.json").read_text())
    return manifest["nodes"][f"model.ingestion.{MODEL}"]


def query_settings(settings: dict[str, object]) -> dict[str, str | int]:
    parsed = {}
    for key, value in settings.items():
        if not isinstance(value, (str, int)):
            raise TypeError(f"unsupported query setting {key}: {value!r}")
        parsed[key] = value
    return parsed


def variants(baseline: dict[str, str | int]) -> dict[str, dict[str, str | int]]:
    return {
        "baseline": baseline,
        "threads4": {**baseline, "max_threads": 4},
        "spill512": {
            **baseline,
            "max_bytes_before_external_group_by": 512 * MIB,
            "max_bytes_before_external_sort": 512 * MIB,
        },
        "join512": {**baseline, "max_bytes_in_join": 512 * MIB},
        "combined": {
            **baseline,
            "max_threads": 4,
            "max_bytes_before_external_group_by": 512 * MIB,
            "max_bytes_before_external_sort": 512 * MIB,
            "max_bytes_in_join": 512 * MIB,
        },
    }


def prepare(client: Client, rows: int, work: Path) -> tuple[str, dict[str, str | int]]:
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
    node = run_dbt(profiles, work / "target")
    sql = node["compiled_code"].strip().rstrip(";")
    baseline = query_settings(node["config"]["query_settings"])
    order_by = ", ".join(node["config"]["order_by"])
    # SAFETY: both fragments come from this repository's compiled dbt model, not external data values.
    # nosemgrep: python.sqlalchemy.security.sqlalchemy-execute-raw-query.sqlalchemy-execute-raw-query
    client.execute(
        f"CREATE TABLE benchmark.expected ENGINE = MergeTree ORDER BY ({order_by}) AS {sql}", settings=baseline
    )
    client.execute("CREATE TABLE benchmark.actual AS benchmark.expected")
    return sql, baseline


def workloads(model_sql: str) -> dict[str, str]:
    return {
        MODEL: model_sql,
        "aggregation": "SELECT group_id, max(payload) AS payload FROM benchmark.input GROUP BY group_id",
        "sort": "SELECT id, payload FROM benchmark.input ORDER BY payload, id",
        "join": "SELECT l.id, r.payload FROM benchmark.input AS l INNER JOIN benchmark.input AS r ON l.id = r.id",
    }


def measure(client: Client, sql: str, settings: dict[str, str | int]) -> dict[str, object]:
    query_id = f"dbt-benchmark-{uuid4()}"
    started = perf_counter()
    client.execute(
        sql,
        settings={
            **settings,
            "query_id": query_id,
            "log_queries": 1,
            "log_queries_probability": 1,
            "log_queries_min_query_duration_ms": 0,
            "use_query_cache": 0,
        },
    )
    elapsed = perf_counter() - started
    logs = []
    for _ in range(20):
        client.execute("SYSTEM FLUSH LOGS")
        logs = client.rows(
            "SELECT query_duration_ms, memory_usage, read_rows, read_bytes, written_rows, "
            "mapFilter((k, v) -> startsWith(k, 'External') OR k IN "
            "('UserTimeMicroseconds', 'SystemTimeMicroseconds'), ProfileEvents) AS events "
            f"FROM system.query_log WHERE query_id = '{query_id}' AND type = 'QueryFinish'"
        )
        if logs:
            break
        sleep(0.1)
    if len(logs) != 1:
        raise RuntimeError(f"expected one query log record for {query_id}, got {len(logs)}")
    return {"wall_seconds": elapsed, **logs[0]}


def benchmark(client: Client, model_sql: str, baseline: dict[str, str | int], repeats: int) -> list[dict[str, object]]:
    results = []
    candidates = variants(baseline)
    rng = random.Random(0)
    for name, sql in workloads(model_sql).items():
        failed = set()
        if name != MODEL:
            client.execute("DROP TABLE benchmark.expected")
            client.execute("DROP TABLE benchmark.actual")
            # SAFETY: sql is one of the constant benchmark workload queries, not external data.
            # nosemgrep: python.sqlalchemy.security.sqlalchemy-execute-raw-query.sqlalchemy-execute-raw-query
            client.execute(
                f"CREATE TABLE benchmark.expected ENGINE = MergeTree ORDER BY tuple() AS {sql}", settings=baseline
            )
            client.execute("CREATE TABLE benchmark.actual AS benchmark.expected")
        for repeat in range(repeats + 1):
            order = list(candidates)
            rng.shuffle(order)
            for variant in order:
                if variant in failed:
                    continue
                settings = candidates[variant]
                client.execute("TRUNCATE TABLE benchmark.actual")
                try:
                    measured = measure(client, f"INSERT INTO benchmark.actual {sql}", settings)
                    client.assert_equal("SELECT * FROM benchmark.actual", "SELECT * FROM benchmark.expected")
                except (RuntimeError, AssertionError) as error:
                    failed.add(variant)
                    results.append(
                        {
                            "workload": name,
                            "variant": variant,
                            "repeat": repeat,
                            "settings": settings,
                            "error": str(error)[:2000],
                        }
                    )
                    sys.stdout.write(f"{name}/{variant}/{repeat}: FAILED: {str(error)[:300]}\n")
                    sys.stdout.flush()
                    continue
                if repeat:
                    results.append(
                        {
                            "workload": name,
                            "variant": variant,
                            "repeat": repeat,
                            "settings": settings,
                            "exact_rows_equal": True,
                            **measured,
                        }
                    )
                    sys.stdout.write(f"{name}/{variant}/{repeat}: {measured['wall_seconds']:.3f}s\n")
                    sys.stdout.flush()
    return results


def insertion(client: Client, baseline: dict[str, str | int], rows: int, repeats: int) -> list[dict[str, object]]:
    results = []
    client.execute(
        "CREATE TABLE benchmark.versions (unique_key UInt64, value UInt64, _version UInt64) "
        "ENGINE = ReplacingMergeTree(_version) ORDER BY unique_key"
    )
    expected = f"SELECT number AS unique_key, number + 2 AS value, toUInt64(2) AS _version FROM numbers({rows})"
    for repeat in range(repeats + 1):
        for optimize in (1, 0) if repeat % 2 else (0, 1):
            client.execute("TRUNCATE TABLE benchmark.versions")
            samples = []
            for version in (1, 2, 2, 1):
                sql = (
                    f"INSERT INTO benchmark.versions SELECT number, number + {version}, toUInt64({version}) "
                    f"FROM numbers({rows})"
                )
                samples.append(measure(client, sql, {**baseline, "optimize_on_insert": optimize}))
                if version == 2:
                    client.assert_equal("SELECT * FROM benchmark.versions FINAL", expected)
            client.assert_equal("SELECT * FROM benchmark.versions FINAL", expected)
            if repeat:
                results.append(
                    {
                        "workload": "insert_dedup",
                        "variant": f"optimize{optimize}",
                        "repeat": repeat,
                        "exact_rows_equal": True,
                        "batches": samples,
                        "wall_seconds": sum(sample["wall_seconds"] for sample in samples),
                    }
                )
    return results


def report(results: list[dict[str, object]], output: Path) -> None:
    groups = sorted({(row["workload"], row["variant"]) for row in results})
    lines = [
        "# Synthetic ClickHouse settings comparison",
        "",
        "SQL execution only; includes writes. Each variant gets an unmeasured warm-up. Exact row comparisons",
        "include duplicates, retries and late versions. Fixed table engines, keys and FINAL reads.",
        "",
        "| Workload | Variant | Median seconds | Peak query MiB |",
        "|---|---|---:|---:|",
    ]
    for workload, variant in groups:
        matching = [row for row in results if row["workload"] == workload and row["variant"] == variant]
        errors = [row for row in matching if "error" in row]
        if errors:
            message = str(errors[0]["error"]).replace("|", " ").replace("\n", " ")[:300]
            lines.append(f"| {workload} | {variant} | FAILED: {message} | — |")
            continue
        samples = [row["wall_seconds"] for row in matching]
        memories = []
        for row in results:
            if row["workload"] == workload and row["variant"] == variant:
                batches = row.get("batches", [row])
                memories.extend(int(batch["memory_usage"]) for batch in batches)
        lines.append(f"| {workload} | {variant} | {median(samples):.3f} | {max(memories) / MIB:.1f} |")
    lines.extend(
        [
            "",
            "The gold workload uses compiled git_authored_commits SQL and its current query settings.",
            "Other workloads isolate spill, sort, join and insert costs; they are not whole-pipeline timings.",
            "One query runs at a time; these results do not prove safety under concurrent dbt builds.",
        ]
    )
    output.write_text("\n".join(lines) + "\n")


def positive(value: str) -> int:
    parsed = int(value)
    if parsed < 1:
        raise argparse.ArgumentTypeError("must be positive")
    return parsed


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--rows", type=positive, required=True)
    parser.add_argument("--repeats", type=positive, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    client = Client.from_env()
    args.output.mkdir(parents=True, exist_ok=True)
    with TemporaryDirectory() as directory:
        sql, baseline = prepare(client, args.rows, Path(directory))
        results = benchmark(client, sql, baseline, args.repeats)
        results.extend(insertion(client, baseline, args.rows, args.repeats))
    metadata = {
        "rows": args.rows,
        "repeats": args.repeats,
        "cpu_count": os.cpu_count(),
        "architecture": platform.machine(),
        "version": client.execute("SELECT version()").strip(),
        "source_sha": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "results": results,
    }
    (args.output / "results.json").write_text(json.dumps(metadata, indent=2) + "\n")
    report(results, args.output / "summary.md")
    if any("error" in row for row in results):
        raise SystemExit("one or more settings variants failed; see the comparison artifact")


if __name__ == "__main__":
    main()
