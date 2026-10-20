from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import random
import subprocess
import sys
from dataclasses import asdict
from pathlib import Path
from tempfile import TemporaryDirectory
from time import perf_counter, sleep
from uuid import uuid4

from client import Client
from experiments import BY_NAME, MODEL, SERVING, Experiment
from fixtures import HERE, READ_PROBES, ROOT, Workload, prepare, targets
from report import write_report


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


def read_probes(client: Client, table: str, settings: dict[str, str | int]) -> list[dict[str, object]]:
    results = []
    for name, predicate in READ_PROBES.items():
        query = f"SELECT count(), sum(contribution) FROM benchmark.{table} WHERE {predicate}"
        expected = f"SELECT count(), sum(contribution) FROM benchmark.expected WHERE {predicate}"
        measured = measure(client, query, settings)
        if client.rows(query) != client.rows(expected):
            raise AssertionError(f"{name} read differs from the expected result")
        results.append({"name": name, "exact_rows_equal": True, **measured})
    return results


def parameters(workload: Workload) -> dict[str, str | int]:
    return {**workload.settings, "partition_by": workload.partition_by}


def benchmark(client: Client, workload: Workload, experiment: Experiment, repeats: int) -> list[dict[str, object]]:
    baseline = parameters(workload)
    candidate = experiment.candidate(baseline)
    targets(client, workload, str(candidate["partition_by"]))
    pair = {"baseline": baseline, "candidate": candidate}
    results = []
    failed = set()
    rng = random.Random(0)
    for repeat in range(repeats + 1):
        order = list(pair)
        rng.shuffle(order)
        for variant in order:
            if variant in failed:
                continue
            config = pair[variant]
            settings = {key: value for key, value in config.items() if key != "partition_by"}
            client.execute(f"TRUNCATE TABLE benchmark.{variant}")
            record = {"workload": workload.name, "variant": variant, "repeat": repeat, "parameters": config}
            try:
                measured = measure(client, f"INSERT INTO benchmark.{variant} {workload.sql}", settings)
                parts = client.scalar(
                    f"SELECT count() FROM system.parts WHERE active AND database = 'benchmark' AND table = '{variant}'"
                )
                client.assert_equal(f"SELECT * FROM benchmark.{variant}", "SELECT * FROM benchmark.expected")
                reads = read_probes(client, variant, workload.settings) if workload.name == SERVING else []
            except (RuntimeError, AssertionError) as error:
                failed.add(variant)
                results.append({**record, "error": str(error)[:2000], "exact_rows_equal": False})
                sys.stdout.write(f"{workload.name}/{variant}/{repeat}: FAILED: {str(error)[:300]}\n")
                sys.stdout.flush()
                continue
            if repeat:
                results.append({**record, "parts": parts, "reads": reads, "exact_rows_equal": True, **measured})
                sys.stdout.write(f"{workload.name}/{variant}/{repeat}: {measured['wall_seconds']:.3f}s\n")
                sys.stdout.flush()
    return results


def insertion(
    client: Client, workload: Workload, experiment: Experiment, rows: int, repeats: int
) -> list[dict[str, object]]:
    optimize = client.scalar("SELECT value FROM system.settings WHERE name = 'optimize_on_insert'")
    baseline = {**workload.settings, "optimize_on_insert": optimize}
    pair = {"baseline": baseline, "candidate": experiment.candidate(baseline)}
    for name in pair:
        client.execute(
            f"CREATE TABLE benchmark.{name} (unique_key UInt64, value UInt64, _version UInt64) "
            "ENGINE = ReplacingMergeTree(_version) ORDER BY unique_key"
        )
    expected = f"SELECT number AS unique_key, number + 2 AS value, toUInt64(2) AS _version FROM numbers({rows})"
    results = []
    failed = set()
    for repeat in range(repeats + 1):
        order = ("baseline", "candidate") if repeat % 2 else ("candidate", "baseline")
        for variant in order:
            if variant in failed:
                continue
            client.execute(f"TRUNCATE TABLE benchmark.{variant}")
            record = {"workload": "insert_dedup", "variant": variant, "repeat": repeat, "parameters": pair[variant]}
            try:
                samples = []
                for version in (1, 2, 2, 1):
                    sql = (
                        f"INSERT INTO benchmark.{variant} SELECT number, number + {version}, toUInt64({version}) "
                        f"FROM numbers({rows})"
                    )
                    samples.append(measure(client, sql, pair[variant]))
                    if version == 2:
                        client.assert_equal(f"SELECT * FROM benchmark.{variant} FINAL", expected)
                client.assert_equal(f"SELECT * FROM benchmark.{variant} FINAL", expected)
            except (RuntimeError, AssertionError) as error:
                failed.add(variant)
                results.append({**record, "error": str(error)[:2000], "exact_rows_equal": False})
                continue
            if repeat:
                results.append(
                    {
                        **record,
                        "exact_rows_equal": True,
                        "batches": samples,
                        "wall_seconds": sum(sample["wall_seconds"] for sample in samples),
                    }
                )
    return results


def positive(value: str) -> int:
    parsed = int(value)
    if parsed < 1:
        raise argparse.ArgumentTypeError("must be positive")
    return parsed


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--experiment", choices=sorted(BY_NAME), required=True)
    parser.add_argument("--rows", type=positive, required=True)
    parser.add_argument("--repeats", type=positive, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    experiment = BY_NAME[args.experiment]
    client = Client.from_env()
    args.output.mkdir(parents=True, exist_ok=True)
    resources = json.loads((args.output / "resources.json").read_text())
    with TemporaryDirectory() as directory:
        workloads = prepare(client, args.rows, Path(directory))
        results = []
        for name in experiment.workloads:
            if name == "insert_dedup":
                results.extend(insertion(client, workloads[MODEL], experiment, args.rows, args.repeats))
            else:
                results.extend(benchmark(client, workloads[name], experiment, args.repeats))
    metadata = {
        "experiment": asdict(experiment),
        "rows": args.rows,
        "repeats": args.repeats,
        "resources": resources,
        "cpu_count": os.cpu_count(),
        "architecture": platform.machine(),
        "version": client.execute("SELECT version()").strip(),
        "seed_sha256": hashlib.sha256((HERE / "seed.sql").read_bytes()).hexdigest(),
        "serving_sha256": hashlib.sha256((HERE / "serving.sql").read_bytes()).hexdigest(),
        "source_sha": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "results": results,
    }
    (args.output / "results.json").write_text(json.dumps(metadata, indent=2) + "\n")
    write_report([metadata], args.output / "summary.md")
    if any("error" in row for row in results):
        raise SystemExit("one or more settings variants failed; see the comparison artifact")


if __name__ == "__main__":
    main()
