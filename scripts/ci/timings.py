from __future__ import annotations

import argparse
import json
from collections.abc import Mapping, Sequence
from datetime import datetime
from pathlib import Path
from typing import Any, TypedDict


class StepTiming(TypedDict):
    name: str
    conclusion: str | None
    execution_seconds: float | None


class JobTiming(TypedDict):
    id: int
    name: str
    conclusion: str | None
    pending_seconds: float | None
    execution_seconds: float | None
    steps: list[StepTiming]


class RunTiming(TypedDict):
    run_id: int
    attempt: int
    workflow: str
    event: str
    head_sha: str
    conclusion: str
    created_at: str
    observed_end_at: str
    elapsed_seconds: float
    jobs: list[JobTiming]


def duration(start: str | None, end: str | None) -> float | None:
    if start is None or end is None:
        return None
    beginning = datetime.fromisoformat(start.replace("Z", "+00:00"))
    ending = datetime.fromisoformat(end.replace("Z", "+00:00"))
    if beginning.tzinfo is None or ending.tzinfo is None:
        raise ValueError("timing timestamps must include a timezone")
    seconds = (ending - beginning).total_seconds()
    if seconds < 0:
        raise ValueError("timing end precedes its start")
    return seconds


def summarize(run: Mapping[str, Any], jobs: Sequence[Mapping[str, Any]]) -> RunTiming:
    if run["status"] != "completed":
        raise ValueError("only completed workflow attempts can be summarized")
    timings: list[JobTiming] = []
    for job in jobs:
        steps: list[StepTiming] = []
        for step in job["steps"]:
            steps.append(
                StepTiming(
                    name=step["name"],
                    conclusion=step["conclusion"],
                    execution_seconds=None
                    if step["conclusion"] == "skipped"
                    else duration(step["started_at"], step["completed_at"]),
                )
            )
        timings.append(
            JobTiming(
                id=job["id"],
                name=job["name"],
                conclusion=job["conclusion"],
                pending_seconds=None
                if job["conclusion"] == "skipped"
                else duration(job.get("created_at"), job["started_at"]),
                execution_seconds=None
                if job["conclusion"] == "skipped"
                else duration(job["started_at"], job["completed_at"]),
                steps=steps,
            )
        )
    elapsed = duration(run["created_at"], run["updated_at"])
    if elapsed is None:
        raise ValueError("completed workflow timestamps must be present")
    return RunTiming(
        run_id=run["id"],
        attempt=run["run_attempt"],
        workflow=run["name"],
        event=run["event"],
        head_sha=run["head_sha"],
        conclusion=run["conclusion"],
        created_at=run["created_at"],
        observed_end_at=run["updated_at"],
        elapsed_seconds=elapsed,
        jobs=timings,
    )


def markdown_cell(value: str) -> str:
    text = value.replace("\n", " ").replace("\r", " ")
    for character in "\\`|[]":
        text = text.replace(character, "\\" + character)
    return text.replace("<", "&lt;").replace(">", "&gt;")


def minutes(seconds: float | None) -> str:
    return "unknown" if seconds is None else f"{seconds / 60:.2f}"


def render(report: RunTiming) -> str:
    lines = [
        "## CI workflow timings",
        "",
        f"{markdown_cell(report['workflow'])}, attempt {report['attempt']}: "
        f"**{minutes(report['elapsed_seconds'])} min** "
        f"({markdown_cell(report['conclusion'])}).",
        "",
        "Elapsed time ends at the completed run's update timestamp. Pending time is "
        "job creation to start and can include dependency and concurrency waits; "
        "it is not a measurement of runner queue time alone. Missing timestamps remain unknown.",
        "",
        "| Job | Result | Pending min | Execution min |",
        "| --- | --- | ---: | ---: |",
    ]
    for job in report["jobs"]:
        result = "unknown" if job["conclusion"] is None else job["conclusion"]
        lines.append(
            f"| {markdown_cell(job['name'])} | {markdown_cell(result)} | "
            f"{minutes(job['pending_seconds'])} | {minutes(job['execution_seconds'])} |"
        )
    return "\n".join(lines) + "\n"


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--event", type=Path, required=True)
    parser.add_argument("--jobs", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--summary", type=Path, required=True)
    args = parser.parse_args()
    event = json.loads(args.event.read_text())
    jobs = [json.loads(line) for line in args.jobs.read_text().splitlines() if line.strip()]
    report = summarize(event["workflow_run"], jobs)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    with args.summary.open("a") as output:
        output.write(render(report))


if __name__ == "__main__":
    main()
