from __future__ import annotations

import argparse
import json
from pathlib import Path
from statistics import median

from experiments import BY_NAME, MIB, SERVING
from fixtures import READ_PROBES
from topology import TOPOLOGIES


def value(rows: list[dict[str, object]], key: str) -> float:
    return median(float(row[key]) for row in rows)


def peak(rows: list[dict[str, object]]) -> float:
    memories = []
    for row in rows:
        batches = row.get("batches", [row])
        memories.extend(int(batch["memory_usage"]) for batch in batches)
    return max(memories) / MIB


def cell(value: object) -> str:
    return str(value).replace("|", " ").replace("\n", " ")[:300]


def write_report(documents: list[dict[str, object]], output: Path) -> None:
    lines = [
        "# Isolated ClickHouse comparisons",
        "",
        "Each experiment has its own fresh CI database and an independent baseline. One parameter changes.",
        "Times are medians after an unmeasured warm-up; memory is peak query MiB. Output equality is exact.",
        "",
        "| Topology | Experiment | Workload | Only change | Baseline s | Candidate s | Faster | Baseline MiB | Candidate MiB |",
        "|---|---|---|---|---:|---:|---:|---:|---:|",
    ]
    reads = [
        "",
        "## Serving reads and parts",
        "",
        "Read probes use fixed settings and SQL shaped like API date filters. These are SQL timings.",
        "",
        "| Topology | Experiment | Probe | Baseline ms | Candidate ms | Baseline parts | Candidate parts |",
        "|---|---|---|---:|---:|---:|---:|",
    ]
    for document in documents:
        experiment = document["experiment"]
        topology = document["topology"]
        results = document["results"]
        for workload in experiment["workloads"]:
            rows = [row for row in results if row["workload"] == workload]
            errors = [row for row in rows if "error" in row]
            label = f"{experiment['parameter']} → {experiment['value']}"
            if errors:
                lines.append(
                    f"| {topology} | {experiment['name']} | {workload} | {cell(label)} | FAILED: {cell(errors[0]['error'])} | — | — | — | — |"
                )
                continue
            baseline = [row for row in rows if row["variant"] == "baseline"]
            candidate = [row for row in rows if row["variant"] == "candidate"]
            before, after = value(baseline, "wall_seconds"), value(candidate, "wall_seconds")
            lines.append(
                f"| {topology} | {experiment['name']} | {workload} | {cell(label)} | {before:.3f} | {after:.3f} | "
                f"{100 * (1 - after / before):+.1f}% | {peak(baseline):.1f} | {peak(candidate):.1f} |"
            )
            probes = set()
            for row in rows:
                probes.update(probe["name"] for probe in row.get("reads", []))
            for probe in sorted(probes):
                timings = []
                for variants in (baseline, candidate):
                    samples = []
                    for row in variants:
                        samples.extend(read["wall_seconds"] for read in row["reads"] if read["name"] == probe)
                    timings.append(median(samples) * 1000)
                reads.append(
                    f"| {topology} | {experiment['name']} | {probe} | {timings[0]:.2f} | {timings[1]:.2f} | "
                    f"{value(baseline, 'parts'):.0f} | {value(candidate, 'parts'):.0f} |"
                )
    lines.extend(reads)
    lines.extend(
        [
            "",
            "## Preparation costs",
            "",
            "Preparation is outside the paired SQL timings.",
            "",
            "| Topology | Experiment | Seed s | dbt compile s |",
            "|---|---|---:|---:|",
        ]
    )
    for document in documents:
        phases = document["phases"]
        lines.append(
            f"| {document['topology']} | {document['experiment']['name']} | "
            f"{phases['seed_seconds']:.2f} | {phases['compile_seconds']:.2f} |"
        )
    lines.extend(
        [
            "",
            "The compiled gold workload preserves FINAL. Partition trials change only disposable gold serving tables.",
            "Application models and runtime configuration are unchanged. Concurrent builds and HTTP API latency are outside this experiment.",
            "Replicated runs retain quorum writes and sequential reads from the pinned application profile; exact outputs are checked directly on both nodes.",
            "Each ClickHouse node has the same CPU and memory cap. The replicated topology has two such nodes plus Keeper and a proxy.",
        ]
    )
    output.write_text("\n".join(lines) + "\n")


def validate(document: dict[str, object]) -> None:
    if document["topology"] not in TOPOLOGIES:
        raise ValueError("unknown benchmark topology")
    nodes = 2 if document["topology"] == "replicated" else 1
    if document["topology_checks"] != {"node_count": nodes, "engines_match": True, "metadata_equal": True}:
        raise ValueError("topology was not checked on every node")
    experiment = BY_NAME[document["experiment"]["name"]]
    results = document["results"]
    if any("error" in row or not row["exact_rows_equal"] for row in results):
        raise ValueError(f"{experiment.name} has failed samples")
    if any(row["checked_nodes"] != nodes for row in results):
        raise ValueError("sample correctness was not checked on every node")
    for workload in experiment.workloads:
        rows = [row for row in results if row["workload"] == workload]
        variants = {}
        for name in ("baseline", "candidate"):
            samples = [row for row in rows if row["variant"] == name]
            if sorted(row["repeat"] for row in samples) != list(range(1, document["repeats"] + 1)):
                raise ValueError(f"{experiment.name}/{workload}/{name} has missing or duplicate repetitions")
            variants[name] = samples[0]["parameters"]
            if any(sample["parameters"] != variants[name] for sample in samples):
                raise ValueError("parameters changed between repetitions")
            if workload == SERVING:
                for sample in samples:
                    probes = sample["reads"]
                    if sorted(probe["name"] for probe in probes) != sorted(READ_PROBES):
                        raise ValueError("missing or duplicate serving read probes")
                    if not all(probe["exact_rows_equal"] for probe in probes):
                        raise ValueError("incorrect serving read results")
                    if any(probe["checked_nodes"] != nodes for probe in probes):
                        raise ValueError("serving reads were not checked on every node")
        if experiment.candidate(variants["baseline"]) != variants["candidate"]:
            raise ValueError(f"{experiment.name}/{workload} changed additional parameters")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input-root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    documents = [json.loads(path.read_text()) for path in sorted(args.input_root.glob("*/results.json"))]
    if not documents:
        raise SystemExit("no experiment reports found")
    write_report(documents, args.output)
    found = [(document["topology"], document["experiment"]["name"]) for document in documents]
    expected = [(topology, name) for topology in TOPOLOGIES for name in BY_NAME]
    if sorted(found) != sorted(expected):
        raise SystemExit(f"missing or duplicate experiment reports: {found}")
    identities = {
        (
            document["source_sha"],
            document["application_sha"],
            document["rows"],
            document["repeats"],
            document["seed_sha256"],
            document["serving_sha256"],
            document["version"],
            document["architecture"],
            json.dumps(document["resources"], sort_keys=True),
        )
        for document in documents
    }
    if len(identities) != 1:
        raise SystemExit("experiments used different code, data, repetitions, server versions or architectures")
    for document in documents:
        validate(document)


if __name__ == "__main__":
    main()
