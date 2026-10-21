from __future__ import annotations

import argparse
import importlib.metadata
import json
import logging
import os
import subprocess
import sys
import time
from pathlib import Path
from statistics import median

VARIANTS = ("baseline", "parse-cache", "retry-waits", "combined")
LOG = logging.getLogger(__name__)
LOOKBACK_REQUEST = (
    "connectors/task-tracking/jira/tests/test_jira_issue_history.py::test_parent_chain_fan_out",
    "cca2a92b9b3409b5e65aca698586dca010c2ef4aaf09f97763bddf0849fa97af",
    200,
)


def comparable_requests(requests: list[list[object]]) -> list[list[object]]:
    result = []
    for case, signature, status, count in requests:
        if (case, signature, status) == LOOKBACK_REQUEST:
            if count not in (1, 2):
                raise ValueError("lookback request must occur once or twice")
            count = 1
        result.append([case, signature, status, count])
    return result


def measured_lines(path: Path) -> dict[str, list[int]]:
    document = json.loads(path.read_text())
    return {name: entry["executed_lines"] for name, entry in document["files"].items()}


def validate(rows: list[dict[str, object]]) -> None:
    baseline = next(row for row in rows if row["variant"] == "baseline")
    if any(row["exit_code"] != 0 or not row["outcomes"] or not row["requests"] for row in rows):
        raise ValueError("missing or unsuccessful test execution")
    for row in rows:
        for key in ("outcomes", "requests", "coverage"):
            observed, expected = row[key], baseline[key]
            if key == "requests":
                observed, expected = comparable_requests(observed), comparable_requests(expected)
            if observed != expected:
                raise ValueError(f"{key} differs in {row['variant']}")


def summary(rows: list[dict[str, object]], error: str = "") -> str:
    lines = [
        "# Isolated connector CI research",
        "",
        "Jira mock suite; same source and installed dependencies; serial repetitions.",
        "",
        "| Change | Median wall s | Manifest CPU s | Schema CPU s | Requested sleeps s |",
        "|---|---:|---:|---:|---:|",
    ]
    for variant in VARIANTS:
        samples = [row for row in rows if row["variant"] == variant]
        if samples:
            values = [
                median(row["wall_seconds"] for row in samples),
                median(row["parse_stats"]["manifest_cpu_seconds"] for row in samples),
                median(row["parse_stats"]["schema_cpu_seconds"] for row in samples),
                median(row["requested_sleep_seconds"] for row in samples),
            ]
            lines.append(f"| {variant} | " + " | ".join(f"{value:.3f}" for value in values) + " |")
    lines += [
        "",
        "Wall time includes pytest startup, collection and coverage. Dependency installation is outside paired totals.",
        "CPU time is used for parsing because tests freeze wall/monotonic clocks. HTTP signatures include method, URL, body and response status; only digests are stored.",
        "Each candidate changes one mechanism; combined is a separate interaction check. Results apply to this suite, not the entire connector matrix.",
        "The existing issue-history fan-out assertion permits lookback partition re-emission. Its one known successful request may occur once or twice; raw counts remain recorded. Every other signature/count, including retries, must match exactly.",
    ]
    if error:
        lines += ["", "FAILED: " + error]
    return "\n".join(lines) + "\n"


def main() -> None:
    logging.basicConfig(level=logging.INFO)
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--repeats", type=int, choices=range(1, 4), required=True)
    parser.add_argument("--start-repeat", type=int, choices=range(1, 4), default=1)
    args = parser.parse_args()
    source, output = args.source.resolve(), args.output.resolve()
    if output.exists():
        raise SystemExit("output must be fresh")
    output.mkdir(parents=True)
    harness = source / "src/ingestion/tests/connectors"
    suite = source / "src/ingestion/connectors/task-tracking/jira/tests"
    document = {
        "application_sha": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=source, text=True).strip(),
        "versions": {
            name: importlib.metadata.version(name) for name in ("airbyte-cdk", "PyYAML", "pytest", "pytest-cov")
        },
        "installed": dict(sorted((item.metadata["Name"], item.version) for item in importlib.metadata.distributions())),
        "samples": [],
    }
    try:
        for repeat in range(args.start_repeat, args.start_repeat + args.repeats):
            offset = (repeat - 1) % len(VARIANTS)
            order = VARIANTS[offset:] + VARIANTS[:offset]
            peers = []
            for variant in order:
                LOG.info("Starting %s/%s", repeat, variant)
                folder = output / f"{repeat}-{variant}"
                folder.mkdir()
                env = os.environ.copy()
                env.update(
                    CONNECTOR_RESEARCH_VARIANT=variant,
                    CONNECTOR_RESEARCH_REPORT=str(folder / "observations.json"),
                    PYTHONHASHSEED="0",
                    PYTHONPATH=os.pathsep.join((str(Path(__file__).resolve().parent), str(harness))),
                )
                command = [
                    sys.executable,
                    "-m",
                    "pytest",
                    str(suite),
                    "-p",
                    "research_plugin",
                    "--cov=connector_tests",
                    f"--cov-report=json:{folder / 'coverage.json'}",
                    "-q",
                ]
                started = time.monotonic()
                with (folder / "pytest.log").open("w") as stream:
                    process = subprocess.run(
                        command, cwd=harness, env=env, stdout=stream, stderr=subprocess.STDOUT, check=False
                    )
                elapsed = time.monotonic() - started
                row = json.loads((folder / "observations.json").read_text())
                row.update(wall_seconds=elapsed, repeat=repeat, coverage=measured_lines(folder / "coverage.json"))
                peers.append(row)
                document["samples"].append(row)
                (output / "results.json").write_text(json.dumps(document, indent=2) + "\n")
                if process.returncode:
                    raise RuntimeError(f"tests failed in {variant}; see {folder / 'pytest.log'}")
            validate(peers)
    except Exception as error:
        document["error"] = str(error)
        raise
    finally:
        (output / "results.json").write_text(json.dumps(document, indent=2) + "\n")
        (output / "summary.txt").write_text(summary(document["samples"], str(document.get("error", ""))))


if __name__ == "__main__":
    main()
