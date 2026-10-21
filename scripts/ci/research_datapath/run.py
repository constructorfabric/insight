from __future__ import annotations

import argparse
import json
import logging
import os
import subprocess
import time
from pathlib import Path

LOG = logging.getLogger(__name__)


def run(command: list[str], source: Path, log: Path, env: dict[str, str]) -> float:
    started = time.monotonic()
    with log.open("w") as stream:
        subprocess.run(command, cwd=source, env=env, stdout=stream, stderr=subprocess.STDOUT, check=True)
    return time.monotonic() - started


def dbt_work(mode: dict[str, object]) -> list[object]:
    result = []
    for row in mode["timings"]:
        if row["stage"].startswith("dbt_"):
            nodes = sorted((node["unique_id"], node["status"]) for node in row.get("nodes", []))
            result.append((row["stage"], row["selector"], nodes))
    if not result:
        raise ValueError("no transformation invocations were measured")
    return result


def main() -> None:
    logging.basicConfig(level=logging.INFO)
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    source, output = args.source.resolve(), args.output.resolve()
    output.mkdir()
    private = output.parent / (output.name + "-private")
    private.mkdir()
    harness = Path(__file__).resolve().parent
    snapshot = output / "gold-snapshot"
    conftest = source / "tests/datapath/metrics/ci/conftest.py"
    if conftest.exists():
        raise ValueError("refusing to replace an existing fixture file")
    data_test = conftest.with_name("test_research_transformation.py")
    if data_test.exists():
        raise ValueError("refusing to replace an existing transformation test")
    data_test.write_text(
        "from snapshot import duration_values\n"
        "def test_gold_values_deduplicate_and_keep_event_grain(spec, instance_cfg):\n"
        "    duration_values(instance_cfg, spec.tenant)\n"
    )
    conftest.write_text("from datapath_research_plugin import spec, _write_coverage_inputs, seeded_gold\n")
    env = {**os.environ, "PYTHONPATH": str(harness), "PYTHONUNBUFFERED": "1"}
    sha = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=source, text=True).strip()
    env["INSIGHT_RESEARCH_SOURCE_SHA"] = sha
    document: dict[str, object] = {"source_sha": sha, "modes": []}
    run_id = os.environ.get("GITHUB_RUN_ID", str(time.time_ns()))
    if not run_id.isdigit():
        raise ValueError("research run ID must be numeric")
    active: list[str] = []
    try:
        for mode in ("baseline", "data", "api"):
            LOG.info("Starting %s", mode)
            folder = output / mode
            folder.mkdir()
            instance = f"ci-split-{run_id}-{mode}"
            existing = subprocess.check_output(
                ["docker", "ps", "-aq", "--filter", f"label=com.docker.compose.project=insight-{instance}"], text=True
            ).strip()
            if existing:
                raise ValueError("refusing to reuse an existing compose project")
            compose = ["./dev-compose.sh", "test-stand"]
            active = [*compose, "down", f"--instance={instance}"]
            boot = run(
                [*compose, "minimal", f"--instance={instance}", "--port-offset=6100"],
                source,
                private / f"{mode}-boot.log",
                env,
            )
            env.update(
                INSIGHT_RESEARCH_MODE=mode,
                INSIGHT_RESEARCH_SOURCE=str(source),
                INSIGHT_RESEARCH_OUTPUT=str(folder),
                INSIGHT_RESEARCH_SNAPSHOT=str(snapshot),
                INSIGHT_RESEARCH_PROJECT=f"insight-{instance}",
            )
            elapsed = run(
                [
                    *compose,
                    "test",
                    f"--instance={instance}",
                    "--tree=tests/datapath/metrics/ci",
                    "-k",
                    "test_gold_values_deduplicate_and_keep_event_grain"
                    if mode == "data"
                    else "test_duration_is_the_median_over_runs_not_the_mean",
                    "-p",
                    "datapath_research_plugin",
                    "-v",
                    "--tb=short",
                    "--log-cli-level=INFO",
                ],
                source,
                folder / "pytest.log",
                env,
            )
            observations = json.loads((folder / "observations.json").read_text())
            expected_replies = 0 if mode == "data" else 3
            passed = [entry for entry in observations["outcomes"] if entry[1:] == ["call", "passed"]]
            if observations["exit_code"] != 0 or len(observations["replies"]) != expected_replies or len(passed) != 3:
                raise ValueError("expected three successful tests with the planned API calls")
            document["modes"].append(
                {"mode": mode, "stand_boot_seconds": boot, "pytest_seconds": elapsed, **observations}
            )
            (output / "results.json").write_text(json.dumps(document, indent=2) + "\n")
            run(active, source, private / f"{mode}-down.log", env)
            active = []
        baseline, data, api = document["modes"]
        if dbt_work(baseline) != dbt_work(data):
            raise ValueError("data lane changed dbt invocations or their node results")
        if any(row["stage"].startswith("dbt_") or row["stage"] == "fixture_warehouse_schema" for row in api["timings"]):
            raise ValueError("API lane unexpectedly invoked the transformation setup")
        for key in ("replies", "outcomes"):
            if baseline[key] != api[key]:
                raise ValueError(f"{key} differ after gold replay")
        for name in ("metric_assertions.json", "metric_definitions.json"):
            if json.loads((output / "baseline" / name).read_text()) != json.loads((output / "api" / name).read_text()):
                raise ValueError(f"{name} differs after gold replay")
        document["validated"] = True
    except Exception as error:
        document["error"] = str(error)
        raise
    finally:
        (output / "results.json").write_text(json.dumps(document, indent=2) + "\n")
        conftest.unlink()
        data_test.unlink()
        if active:
            run(active, source, private / "cleanup.log", env)


if __name__ == "__main__":
    main()
