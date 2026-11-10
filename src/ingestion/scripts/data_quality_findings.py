"""Emit one structured finding per data-quality check from dbt's artifacts.

Reads target/run_results.json and target/manifest.json relative to the cwd
(the dbt project dir), not dbt's logger, so each finding is one line in the
agreed shape — queryable as `| json | fields_event="data_quality_finding"`
(Loki flattens nested keys with `fields_`). One `data_quality_run` line per
invocation then reports whether the run itself worked, apart from any finding.
"""

from __future__ import annotations

import argparse
import json
import logging
import sys
from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path

import insight_logging

QUALITY_TAGS = frozenset({"data_quality", "connector_quality"})


class Scope(StrEnum):
    POST_SYNC = "post_sync"
    SCHEDULED = "scheduled"


@dataclass(frozen=True)
class Invocation:
    scope: Scope
    connector: str
    dbt_exit_code: int


def parse_args(argv: list[str]) -> Invocation:
    parser = argparse.ArgumentParser()
    parser.add_argument("--scope", type=Scope, choices=list(Scope), required=True)
    parser.add_argument("--connector", default="")
    parser.add_argument("--dbt-exit-code", type=int, required=True)
    args = parser.parse_args(argv)
    return Invocation(scope=args.scope, connector=args.connector, dbt_exit_code=args.dbt_exit_code)


def findings_logger() -> logging.Logger:
    log = insight_logging.configure("dbt")
    # INVARIANT: the Grafana data-quality rules resolve on the latest finding and
    # run line, passes included, so these lines must survive a warn/error install level.
    log.setLevel(logging.INFO)
    return log


def emit_findings(log: logging.Logger, invocation: Invocation, run_results: dict, nodes: dict) -> int:
    run_id = run_results["metadata"].get("invocation_id")
    errored = 0

    for result in run_results["results"]:
        node = nodes.get(result["unique_id"])
        if not node or node.get("resource_type") != "test":
            continue
        # `connector_quality` is the per-connector catalog: checks that need a
        # connector's own bronze/staging and so cannot live in `data_quality`,
        # which must pass on a tenant where that connector is absent. Findings
        # from both carry the same shape.
        if not (QUALITY_TAGS & set(node.get("tags") or [])):
            continue

        status = result.get("status")
        if status == "error":
            errored += 1

        config = node.get("config") or {}
        meta = config.get("meta") or {}
        level = log.info if status == "pass" else log.warning
        level(
            "data quality finding",
            extra={
                "event": "data_quality_finding",
                "run_id": run_id,
                "scope": invocation.scope.value,
                "connector": invocation.connector,
                "check_id": node.get("name"),
                "title": meta.get("title", node.get("name")),
                "domain": meta.get("domain", "unknown"),
                "category": meta.get("category", "uncategorized"),
                "gate": config.get("severity"),
                "tier": meta.get("tier", "warn"),
                "status": status,
                "rows_violating": result.get("failures") or 0,
                "duration_ms": round((result.get("execution_time") or 0) * 1000),
                "audit_relation": result.get("relation_name"),
                "remediation": meta.get("remediation"),
            },
        )
    return errored


def emit_run(log: logging.Logger, invocation: Invocation, run_id: str | None, errored: int, reason: str | None) -> None:
    failed = reason is not None
    level = log.error if failed else log.info
    level(
        "data quality run failed" if failed else "data quality run completed",
        extra={
            "event": "data_quality_run",
            "run_id": run_id,
            "scope": invocation.scope.value,
            "connector": invocation.connector,
            "dbt_exit_code": invocation.dbt_exit_code,
            "checks_errored": errored,
            "failed": int(failed),
            "reason": reason,
        },
    )


def failure_reason(invocation: Invocation, errored: int) -> str | None:
    if errored:
        return f"{errored} check(s) errored instead of reporting a result"
    if invocation.dbt_exit_code != 0:
        return f"dbt test exited {invocation.dbt_exit_code}"
    return None


def main(argv: list[str]) -> int:
    invocation = parse_args(argv)
    log = findings_logger()

    try:
        run_results = json.loads(Path("target/run_results.json").read_text())
        nodes = json.loads(Path("target/manifest.json").read_text())["nodes"]
    except (OSError, ValueError, KeyError) as exc:
        emit_run(log, invocation, None, 0, f"dbt left no readable artifacts ({type(exc).__name__}); no check ran")
        return 0

    errored = emit_findings(log, invocation, run_results, nodes)
    run_id = run_results["metadata"].get("invocation_id")
    emit_run(log, invocation, run_id, errored, failure_reason(invocation, errored))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
