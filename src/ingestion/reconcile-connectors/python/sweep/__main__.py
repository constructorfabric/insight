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
    #: Absent for a connector the controller manages but the mover has no
    #: connection for yet. It is still configured — that is the first thing the
    #: page answers — it simply has nothing to read.
    connection_id: str | None


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

    # INVARIANT: every entry or none. Skipping a malformed one would seal a
    # SHORT list as this tick's complete snapshot, and the read side treats a
    # sealed snapshot as authoritative — so the connector that fell out would
    # render as no longer configured rather than as unread.
    connectors = []
    for position, item in enumerate(raw):
        if not isinstance(item, dict):
            raise UnreadableWork(f"connector {position} is not an object")
        name = item.get("name")
        if not isinstance(name, str) or not name:
            raise UnreadableWork(f"connector {position} carries no name")
        connection_id = item.get("connection_id")
        if connection_id is not None and not isinstance(connection_id, str):
            raise UnreadableWork(f"connector {name!r} carries an unusable connection id")
        connectors.append(Connector(name, connection_id or None))
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

    # Two different sets, deliberately. The connection map turns a job into a
    # connector, so it holds only connectors the mover has a connection for; a
    # job on a connection absent from it is skipped rather than guessed at. The
    # configured set is every connector the controller manages, whether or not
    # the mover has caught up — a connector awaiting its first connection is
    # configured and has never synced, which is a state the page must be able to
    # show.
    by_connection = {c.connection_id: c.name for c in connectors if c.connection_id}
    configured = [c.name for c in connectors]

    read_failed = False
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

        # The read start is floored, so a job open longer than that floor will
        # never be asked about again. Say so, rather than leaving its last
        # provisional word standing as the page's answer.
        stranded = plan.plan_abandoned(ledger.abandoned_jobs(watermark), tick_id)
        if stranded:
            _log(
                f"{len(stranded)} job(s) have fallen below the read start; "
                "recording their state as unreadable"
            )
            written += ledger.insert(stranded)
    # Every failure, not only the two typed ones: an unanticipated shape in the
    # listing would otherwise escape and skip the seal by accident rather than by
    # decision, which is the same outcome reached without the reasoning.
    except Exception as error:  # noqa: BLE001
        read_failed = True
        _log(f"could not read this tick's syncs: {error!r}")

    # INVARIANT: the seal is what dates the page — the read surface reports the
    # newest sealed tick as "when the mover was last read". A tick that never
    # read the mover must not seal, or an install whose mover is unreachable
    # keeps reporting that it was just checked, and the page can never say
    # recording has stopped.
    if read_failed:
        _log("the mover was not read; leaving this tick unsealed")
        return 1

    try:
        written += ledger.insert(plan.plan_snapshot(configured, tick_id))
        written += ledger.insert([plan.plan_seal(tick_id)])
    except LedgerError as error:
        _log(f"cannot seal this tick: {error}")
        return 1

    _log(f"tick {tick_id}: {written} rows across {len(configured)} connectors")
    return 1 if incomplete else 0


if __name__ == "__main__":
    sys.exit(run(sys.stdin))
