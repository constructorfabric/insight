from __future__ import annotations

import argparse
import json
import os
import subprocess
import time
from pathlib import Path


def run(command: list[str], source: Path, log: Path, env: dict[str, str]) -> float:
    started = time.monotonic()
    with log.open("w") as stream:
        subprocess.run(command, cwd=source, env=env, stdout=stream, stderr=subprocess.STDOUT, check=True)
    return time.monotonic() - started


def main() -> None:
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
    conftest.write_text("from fixtures import spec, _write_coverage_inputs, seeded_gold\n")
    env = {**os.environ, "PYTHONPATH": str(harness), "PYTHONUNBUFFERED": "1"}
    sha = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=source, text=True).strip()
    env["INSIGHT_RESEARCH_SOURCE_SHA"] = sha
    document: dict[str, object] = {"source_sha": sha, "modes": []}
    active: list[str] = []
    try:
        for mode in ("baseline", "api"):
            folder = output / mode
            folder.mkdir()
            instance = f"ci-split-{mode}"
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
                    "test_duration_is_the_median_over_runs_not_the_mean",
                    "-p",
                    "fixtures",
                    "-v",
                    "--tb=short",
                    "--log-cli-level=INFO",
                ],
                source,
                folder / "pytest.log",
                env,
            )
            observations = json.loads((folder / "observations.json").read_text())
            if observations["exit_code"] != 0 or len(observations["replies"]) != 3:
                raise ValueError("expected three successful original API tests")
            document["modes"].append(
                {"mode": mode, "stand_boot_seconds": boot, "pytest_seconds": elapsed, **observations}
            )
            (output / "results.json").write_text(json.dumps(document, indent=2) + "\n")
            run(active, source, private / f"{mode}-down.log", env)
            active = []
        baseline, api = document["modes"]
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
        if active:
            run(active, source, private / "cleanup.log", env)


if __name__ == "__main__":
    main()
