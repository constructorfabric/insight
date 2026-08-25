#!/usr/bin/env python3
"""Assemble one sweep tick's plan request (connector-health spec §3.2).

Pure over values: turns what the shell gathered — the mover's jobs, the
connection map, the ledger's resolved state, the workflow records' claims — into
the request `sweep_plan.py` consumes. Separate from the planner so the shell
carries no parsing and neither file carries two concerns.

    sweep_request.py <jobs> <mapping> <ledger> <claims> <tick_run_id> <horizon_epoch>

Every positional but the last is JSON. Writes the request to stdout.
"""

from __future__ import annotations

import datetime
import json
import sys
from typing import Any


def epoch(timestamp: str | None) -> int:
    """Epoch seconds from the mover's ISO-8601 stamp; 0 when absent."""
    if not timestamp:
        return 0
    try:
        return int(datetime.datetime.fromisoformat(timestamp).timestamp())
    except ValueError:
        return 0


def ledger_row(raw: dict[str, str]) -> dict[str, Any]:
    """Typed row from the warehouse's all-strings map output.

    `has_counters` is true for a sweep-origin row: counters come from the
    mover's history, so the pipeline's own row — which carries the claim and the
    delivery measurement — does not mark a job as collected.
    """
    return {
        "job_id": raw["job_id"],
        "connector": raw["connector"],
        "claim": raw["claim"],
        "status": raw["status"],
        "has_counters": raw["has_counters"] == "1",
        "started_at_epoch": int(raw["started_at_epoch"]),
        "duration_ms": int(raw["duration_ms"]),
        "records_moved": int(raw["records_moved"]),
    }


def build(
    jobs: list[dict[str, Any]],
    mapping: dict[str, Any],
    ledger: list[dict[str, str]],
    claims: dict[str, Any],
    tick_run_id: str,
    horizon_epoch: int,
) -> dict[str, Any]:
    for job in jobs:
        job["startTimeEpoch"] = epoch(job.get("startTime"))

    return {
        "jobs": jobs,
        "connections": mapping.get("connections", {}),
        "configured": mapping.get("configured", []),
        "ledger": [ledger_row(row) for row in ledger],
        "workflow_claims": claims.get("claims", {}),
        "records_readable": bool(claims.get("readable")),
        # A duration, not an observation: see sweep_claims for why the oldest
        # surviving record is not a floor.
        "horizon_epoch": horizon_epoch,
        "tick_run_id": tick_run_id,
    }


def main() -> int:
    jobs, mapping, ledger, claims, tick_run_id, horizon_epoch = sys.argv[1:7]
    json.dump(
        build(
            json.loads(jobs),
            json.loads(mapping),
            json.loads(ledger),
            json.loads(claims),
            tick_run_id,
            int(horizon_epoch),
        ),
        sys.stdout,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
