#!/usr/bin/env python3
"""Assemble one sweep tick's plan request (connector-health spec §3.2).

Pure over values: turns what the shell gathered — the mover's jobs, the
connection map, the ledger's resolved state, the workflow records' claims — into
the request `sweep_plan.py` consumes. Separate from the planner so the shell
carries no parsing and neither file carries two concerns.

    sweep_request.py <tick_run_id> <horizon_epoch>  < envelope.json

The four gathered documents arrive as one JSON envelope on stdin — keyed
`jobs`, `mapping`, `ledger`, `claims`. They are unbounded (a first sweep reads
the mover's whole retained history), and an argument vector is not: past the
kernel's limit `execve` fails with E2BIG before Python starts, which would
strand the sweep exactly on the installations with the most to record.

Writes the request to stdout.
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
        # A sweep row that already carries a terminal outcome. Separate from
        # `has_counters`, which only says some sweep row exists.
        "collected": raw.get("collected") == "1",
        # "0" is the warehouse saying NULL: a job the mover had not started.
        "started_at_epoch": int(raw["started_at_epoch"] or 0),
        # Empty means the warehouse held NULL: nobody timed this one.
        "duration_ms": int(raw["duration_ms"]) if raw["duration_ms"] else None,
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
        # Two different facts, kept apart. `startTime` is when the sync began
        # and is absent for a job the mover has not started; `createdAt` is what
        # the listing is ordered by, and so the axis the frontier moves along.
        # Substituting one for the other would report a start that never
        # happened and still leave the cursor on the wrong axis.
        job["startTimeEpoch"] = epoch(job.get("startTime"))
        job["createdAtEpoch"] = epoch(job.get("createdAt"))

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
    tick_run_id, horizon_epoch = sys.argv[1:3]
    envelope = json.load(sys.stdin)
    json.dump(
        build(
            envelope.get("jobs") or [],
            envelope.get("mapping") or {},
            envelope.get("ledger") or [],
            envelope.get("claims") or {},
            tick_run_id,
            int(horizon_epoch),
        ),
        sys.stdout,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
