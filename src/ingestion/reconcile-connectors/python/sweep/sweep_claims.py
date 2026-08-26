#!/usr/bin/env python3
"""Read job claims out of the workflow layer's records (connector-health §3.2).

Pure over values: takes a workflow listing on stdin, prints which mover job each
retained run claims.

A run claims a job by exact identity — the sync step exposes the job it
triggered as its own result. Timing is never used: a manual sync may run while a
pipeline run is mid-transform, so overlapping wall-clock proves nothing.

Deliberately does NOT derive how far back the records reach. Retention there may
instance, retention is uneven: a few ancient runs survive while whole recent days
are collected, so the oldest surviving record is not a floor under anything. The
caller supplies the horizon as a duration instead.
"""

from __future__ import annotations

import json
import sys
from typing import Any

# The node whose result is the mover job id the run triggered.
TRIGGER_NODE = "trigger"


def claimed_job(workflow: dict[str, Any]) -> str:
    for node in ((workflow.get("status") or {}).get("nodes") or {}).values():
        if node.get("displayName") != TRIGGER_NODE:
            continue
        job_id = str((node.get("outputs") or {}).get("result") or "").strip()
        if job_id:
            return job_id
    return ""


def read_claims(payload: dict[str, Any]) -> dict[str, Any]:
    claims: dict[str, str] = {}
    for workflow in payload.get("items") or []:
        job_id = claimed_job(workflow)
        if job_id:
            claims[job_id] = workflow["metadata"]["name"]

    return {
        "claims": claims,
        # Readable, not "non-empty": an instance retaining no workflows still
        # answers the question — it answers it with nothing.
        "readable": True,
    }


UNREADABLE = {"claims": {}, "readable": False}


def main() -> int:
    try:
        payload = json.load(sys.stdin)
    except (json.JSONDecodeError, UnicodeDecodeError):
        json.dump(UNREADABLE, sys.stdout)
        return 0
    json.dump(read_claims(payload), sys.stdout)
    return 0


if __name__ == "__main__":
    sys.exit(main())
