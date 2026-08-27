"""One sweep tick: copy the mover's account, record the configured set, seal.

Reads its work from stdin as JSON so no connector list or connection id ever
lands in argv:

    {"tick_id": "...",
     "connectors": [{"name": "example-tracker", "connection_id": "..."}]}

Exits non-zero on any failure. The caller in the reconcile loop swallows that
deliberately and visibly — observability is subordinate to the thing observed,
and no tick may abort because recording broke.
"""

from __future__ import annotations

import json
import sys
from typing import Any, NamedTuple

from . import plan
from .ledger import Ledger, LedgerError
from .mover import Mover, MoverError


class UnreadableWork(ValueError):
    """The tick's own instructions could not be read."""


class Connector(NamedTuple):
    name: str
    connection_id: str


def _log(message: str) -> None:
    print(f"sweep: {message}", file=sys.stderr, flush=True)


def _read_work(stream: Any) -> tuple[str, list[Connector]]:
    try:
        work = json.load(stream)
    except json.JSONDecodeError as error:
        raise UnreadableWork(f"work is not JSON: {error}") from error
    if not isinstance(work, dict):
        raise UnreadableWork("work is not an object")

    tick_id = work.get("tick_id")
    if not isinstance(tick_id, str) or not tick_id.strip():
        raise UnreadableWork("work carries no tick_id")

    raw = work.get("connectors")
    if not isinstance(raw, list):
        raise UnreadableWork("work carries no connectors array")

    connectors = []
    for item in raw:
        if not isinstance(item, dict):
            continue
        name = item.get("name")
        connection_id = item.get("connection_id")
        if isinstance(name, str) and name and isinstance(connection_id, str):
            connectors.append(Connector(name, connection_id))
    return tick_id.strip(), connectors


def run(stream: Any) -> int:
    try:
        tick_id, connectors = _read_work(stream)
    except UnreadableWork as error:
        _log(f"cannot read this tick's work: {error}")
        return 1

    # INVARIANT: an empty configured set is indistinguishable from "everything
    # was removed", so a tick with nothing to record records nothing at all —
    # not an empty snapshot, and no seal to make one readable.
    if not connectors:
        _log("no connectors resolved; recording nothing rather than an empty set")
        return 1

    try:
        ledger = Ledger.from_env()
        mover = Mover.from_env()
    except (LedgerError, MoverError) as error:
        _log(f"cannot reach the inputs: {error}")
        return 1

    # The connection map is what turns a job into a connector. A job on a
    # connection absent from it is skipped rather than guessed at.
    by_connection = {c.connection_id: c.name for c in connectors}

    incomplete = False
    written = 0
    try:
        watermark = ledger.watermark()
        closed = ledger.closed_job_ids(watermark)
        entries, truncated = mover.sync_jobs(plan.as_listing_stamp(watermark))
        if truncated:
            incomplete = True
            _log(
                "history deeper than one tick may read; recorded what was "
                "reached and will continue from that edge next tick"
            )

        planned = plan.plan_syncs(entries, by_connection, tick_id, closed)
        for refusal in planned.skipped:
            _log(f"skipped job {refusal.job_id or '<unnamed>'}: {refusal.reason}")

        written += ledger.insert(planned.rows)
    except (LedgerError, MoverError) as error:
        # The configured set is a fact about configuration, not about whether
        # the listing answered, so it is still worth sealing — but the caller
        # learns the tick was incomplete.
        incomplete = True
        _log(f"could not record this tick's syncs: {error}")

    try:
        written += ledger.insert(plan.plan_snapshot(by_connection.values(), tick_id))
        written += ledger.insert([plan.plan_seal(tick_id)])
    except LedgerError as error:
        _log(f"cannot seal this tick: {error}")
        return 1

    _log(f"tick {tick_id}: {written} rows across {len(connectors)} connectors")
    return 1 if incomplete else 0


if __name__ == "__main__":
    sys.exit(run(sys.stdin))
