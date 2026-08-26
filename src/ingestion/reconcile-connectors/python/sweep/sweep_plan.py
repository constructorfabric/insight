#!/usr/bin/env python3
"""Plan one sweep tick's ledger rows (connector-health spec §3.2, Job Sweep).

Pure over values: given what the mover, the ledger, the workflow layer and the
descriptor set say, decide which rows to insert. No connection, no filesystem —
`lib/sweep.sh` gathers the inputs and performs the insert.

Reads a plan request as JSON on stdin, writes the rows to insert as JSON on
stdout:

    {
      "jobs":            [{"jobId", "connectionId", "status", "startTime",
                           "duration", "rowsSynced", "bytesSynced"}],
      "connections":     {"<connectionId>": "<connector>"},
      "ledger":          [{"job_id", "claim", "has_counters", "started_at_epoch"}],
      "workflow_claims": {"<job_id>": "<run_id>"},
      "records_readable": true,
      "configured":      ["<connector>"],
      "tick_run_id":     "<workflow name of this tick>",
      "horizon_epoch":   1700000000
    }
"""

from __future__ import annotations

import json
import sys
from dataclasses import asdict, dataclass, field
from typing import Any

CLAIMED = "claimed"
OUT_OF_BAND = "out_of_band"
UNCLAIMED = "unclaimed"

SYNC_COMPLETED = "sync.completed"
CONNECTOR_CONFIGURED = "connector.configured"
SWEEP_COMPLETED = "sweep.completed"

ORIGIN = "sweep"

# The mover's outcome vocabulary, mapped to the ledger's at this boundary.
STATUS_FROM_MOVER = {
    "succeeded": "ok",
    "failed": "failed",
    "cancelled": "cancelled",
    "running": "running",
    "incomplete": "failed",
    "pending": "running",
}

# The outcomes that END a job. Everything else — `running`, a queue state, or a
# word this build has never seen — is provisional.
#
# INVARIANT: fail-closed. Listing the non-terminal words instead would let any
# status the mover adds later close a job forever: the tick that could record
# its real outcome skips it as already collected, and the page shows that sync
# unfinished for as long as the ledger keeps it. Re-recording a job one extra
# time costs a row; missing its outcome costs the truth.
TERMINAL_STATUSES = frozenset({"ok", "failed", "cancelled"})


@dataclass(frozen=True)
class Row:
    """One ledger row to insert. Field names are the ledger's column names."""

    event: str
    connector: str = ""
    run_id: str = ""
    job_id: str = ""
    status: str = ""
    claim: str = ""
    origin: str = ORIGIN
    started_at_epoch: int = 0
    created_at_epoch: int = 0
    duration_ms: int | None = None
    records_moved: int = 0


@dataclass
class Plan:
    rows: list[Row] = field(default_factory=list)
    #: The marker that seals this tick, kept OUT of `rows` on purpose.
    #:
    #: Every snapshot read keys on the newest sealed tick, so the seal must land
    #: last — after the rows above AND after the storage observations the shell
    #: writes separately. Sealing first names a tick whose observations do not
    #: exist yet, and every reader in that window sees blank storage for every
    #: connector.
    seal: Row | None = None
    # Jobs whose connection no longer exists: nothing the page could attribute,
    # so they are reported rather than recorded under an empty connector.
    unmappable_jobs: list[str] = field(default_factory=list)
    #: Jobs whose start time the mover reported in a shape we cannot read. They
    #: are left uncovered rather than recorded at the epoch, which the page
    #: would render as a real sync in 1970.
    undatable_jobs: list[str] = field(default_factory=list)


#: What the ledger stores for a word outside the mover's documented vocabulary.
#:
#: The read surface reasons over a closed set — the frontend maps each word to a
#: state and refuses to call an unreadable one healthy. Passing a vendor word
#: straight through would put a value in the column that nothing downstream has
#: a reading for, so the boundary names the unknown instead.
UNKNOWN_STATUS = "unknown"


def ledger_status(mover_status: str) -> str:
    """The ledger's word for a mover outcome. Closed set in, closed set out."""
    return STATUS_FROM_MOVER.get(str(mover_status).lower(), UNKNOWN_STATUS)


def duration_ms(duration: Any) -> int | None:
    """Milliseconds from the mover's ISO-8601 duration (`PT1M37S`) or a number.

    None when the mover reported none or reported it in a shape we cannot read
    — the page states elapsed time as a fact, and a zero would be one.
    """
    if duration is None or duration == "":
        return None
    if isinstance(duration, (int, float)):
        return int(float(duration) * 1000)

    text = str(duration).strip().upper()
    if not text.startswith("PT"):
        return None

    units = {"H": 3600, "M": 60, "S": 1}
    seconds = 0.0
    number = ""
    saw_unit = False
    for char in text[2:]:
        if char.isdigit() or char == ".":
            number += char
            continue
        # A unit this build does not know, or one with no number in front of it,
        # means the shape is not what we think. Returning the partial sum would
        # report a duration shorter than the truth as if it were measured.
        if char not in units or not number:
            return None
        seconds += float(number) * units[char]
        number = ""
        saw_unit = True

    # Trailing digits with no unit, or `PT` with nothing after it.
    if number or not saw_unit:
        return None

    return int(seconds * 1000)


def claim_for(
    job_id: str, workflow_claims: dict[str, str], records_readable: bool, started_at_epoch: int, horizon_epoch: int
) -> tuple[str, str]:
    """The claim this tick can justify, and the run that claims it.

    Only job identity claims. Concurrent timing is never evidence, so absence of
    a claim means out-of-band under exactly one condition: the records that
    could have claimed it were readable AND still reach back that far. Past the
    horizon the records are gone, so their silence proves nothing and the job
    stays unclaimed rather than being called manual.
    """
    run_id = workflow_claims.get(job_id, "")
    if run_id:
        return CLAIMED, run_id
    if not records_readable:
        return UNCLAIMED, ""
    if horizon_epoch and started_at_epoch and started_at_epoch < horizon_epoch:
        return UNCLAIMED, ""
    return OUT_OF_BAND, ""


def coverage_rows(request: dict[str, Any], covered: set[str], plan: Plan) -> None:
    """A sync.completed row for every job the ledger does not hold at all."""
    connections = request.get("connections") or {}
    workflow_claims = request.get("workflow_claims") or {}
    records_readable = bool(request.get("records_readable"))
    horizon = int(request.get("horizon_epoch") or 0)

    for job in request.get("jobs") or []:
        job_id = str(job.get("jobId", ""))
        if not job_id or job_id in covered:
            continue

        connector = connections.get(str(job.get("connectionId", "")), "")
        if not connector:
            plan.unmappable_jobs.append(job_id)
            continue

        started_at = int(job.get("startTimeEpoch") or 0)
        created_at = int(job.get("createdAtEpoch") or 0)
        if created_at <= 0:
            # Nothing places this job on the axis the frontier moves along, so
            # recording it would let the cursor pass jobs nothing has read.
            plan.undatable_jobs.append(job_id)
            continue

        # A job the mover has not started yet has no start time. That is
        # recorded as absent; the claim decision then has no stamp to test and
        # the job stays unclaimed until a later tick can settle it.
        claim, run_id = claim_for(job_id, workflow_claims, records_readable, started_at, horizon)
        plan.rows.append(
            Row(
                event=SYNC_COMPLETED,
                connector=connector,
                run_id=run_id,
                job_id=job_id,
                status=ledger_status(job.get("status", "")),
                claim=claim,
                started_at_epoch=started_at,
                created_at_epoch=created_at,
                duration_ms=duration_ms(job.get("duration")),
                records_moved=int(job.get("rowsSynced") or 0),
            )
        )


def corroboration_rows(request: dict[str, Any], plan: Plan) -> None:
    """A superseding row for every still-unclaimed job the records can settle.

    Retried every tick while the job stays inside the workflow-record horizon,
    so a temporarily unreachable record store delays a claim rather than
    freezing one. Past the horizon the question is closed and unclaimed stands.
    """
    workflow_claims = request.get("workflow_claims") or {}
    records_readable = bool(request.get("records_readable"))
    if not records_readable:
        return

    horizon = int(request.get("horizon_epoch") or 0)
    connections = request.get("connections") or {}

    for row in request.get("ledger") or []:
        if row.get("claim") != UNCLAIMED:
            continue
        if int(row.get("started_at_epoch") or 0) < horizon:
            continue

        job_id = str(row.get("job_id", ""))
        started_at = int(row.get("started_at_epoch") or 0)
        claim, run_id = claim_for(job_id, workflow_claims, records_readable, started_at, horizon)
        if claim == UNCLAIMED:
            continue

        plan.rows.append(
            Row(
                event=SYNC_COMPLETED,
                connector=row.get("connector", "") or connections.get(str(row.get("connection_id", "")), ""),
                run_id=run_id,
                job_id=job_id,
                status=row.get("status", ""),
                claim=claim,
                started_at_epoch=started_at,
                duration_ms=row.get("duration_ms"),
                records_moved=int(row.get("records_moved") or 0),
            )
        )


def snapshot_rows(request: dict[str, Any], plan: Plan) -> None:
    """The configured set, plus the marker that seals it.

    The marker is what makes an empty snapshot representable: without it,
    removing the last connector would leave the previous snapshot authoritative.
    It is returned separately from `rows` so the caller writes it last — see
    `Plan.seal`.
    """
    tick_run_id = str(request.get("tick_run_id", ""))
    for connector in request.get("configured") or []:
        plan.rows.append(Row(event=CONNECTOR_CONFIGURED, connector=connector, run_id=tick_run_id, status="ok"))
    plan.seal = Row(event=SWEEP_COMPLETED, run_id=tick_run_id, status="ok")


def plan_sweep(request: dict[str, Any]) -> Plan:
    plan = Plan()
    covered = {
        str(row.get("job_id", ""))
        for row in request.get("ledger") or []
        if row.get("has_counters") and row.get("status") in TERMINAL_STATUSES
    }

    coverage_rows(request, covered, plan)
    corroboration_rows(request, plan)
    snapshot_rows(request, plan)
    return plan


def main() -> int:
    request = json.load(sys.stdin)
    plan = plan_sweep(request)
    json.dump(
        {
            "rows": [asdict(row) for row in plan.rows],
            "seal": asdict(plan.seal) if plan.seal else None,
            "unmappable_jobs": plan.unmappable_jobs,
            "undatable_jobs": plan.undatable_jobs,
        },
        sys.stdout,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
