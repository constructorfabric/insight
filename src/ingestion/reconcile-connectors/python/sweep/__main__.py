"""One sweep tick: copy the mover's account, record the configured set, seal.

Reads its work from stdin as JSON so no connector list or connection id ever
lands in argv:

    {"tick_id": "...",
     "connectors":  [{"name": "example-tracker", "tenant_id": "...",
                      "source_id": "main", "connection_id": "..."}],
     "descriptors": [{"name": "example-tracker",
                      "namespace": "bronze_example_tracker"}]}

`descriptors` covers every connector this build ships, installed or not: it is
what lets history whose Secret is gone be handed back to the instance that
recorded it.

Exits non-zero on any failure. The caller in the reconcile loop swallows that
deliberately and visibly — observability is subordinate to the thing observed,
and no tick may abort because recording broke.
"""

from __future__ import annotations

import json
import logging
import os
import sys
from typing import Any, NamedTuple

import insight_logging

from . import heal, plan
from .ledger import Ledger, LedgerError
from .mover import Mover, MoverError

_LOG = logging.getLogger("sweep")


class UnreadableWork(ValueError):
    """The tick's own instructions could not be read."""


class UnfilteredListing(RuntimeError):
    """The mover served a listing it did not apply the watermark to."""


class Connector(NamedTuple):
    name: str
    tenant_id: str
    source_id: str
    #: Absent for a connector the controller manages but the mover has no
    #: connection for yet. It is still configured — that is the first thing the
    #: page answers — it simply has nothing to read.
    connection_id: str | None

    @property
    def instance(self) -> plan.Instance:
        """What the ledger records this connector's rows under."""
        return plan.Instance(self.name, self.tenant_id, self.source_id)


def _read_work(stream: Any) -> tuple[str, list[Connector], dict[str, str]]:
    try:
        work = json.load(stream)
    except json.JSONDecodeError as error:
        raise UnreadableWork(f"work is not JSON: {error}") from error
    if not isinstance(work, dict):
        raise UnreadableWork("work is not an object")

    tick_id = work.get("tick_id")
    if not isinstance(tick_id, str) or not tick_id.strip():
        raise UnreadableWork("work carries no tick_id")

    namespaces: dict[str, str] = {}
    for position, item in enumerate(work.get("descriptors") or []):
        if not isinstance(item, dict):
            raise UnreadableWork(f"descriptor {position} is not an object")
        shipped, namespace = item.get("name"), item.get("namespace")
        if isinstance(shipped, str) and shipped and isinstance(namespace, str):
            namespaces[shipped] = namespace

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
        # SAFETY: required, never defaulted. A row recorded under a guessed
        # identity cannot be told from one recorded under a real one, and the
        # page would attribute an instance's syncs to a sibling.
        tenant_id = item.get("tenant_id")
        if not isinstance(tenant_id, str) or not tenant_id:
            raise UnreadableWork(f"connector {name!r} carries no tenant id")
        source_id = item.get("source_id")
        if not isinstance(source_id, str) or not source_id:
            raise UnreadableWork(f"connector {name!r} carries no source id")
        connection_id = item.get("connection_id")
        if connection_id is not None and not isinstance(connection_id, str):
            raise UnreadableWork(f"connector {name!r} carries an unusable connection id")
        connectors.append(Connector(name, tenant_id, source_id, connection_id or None))
    return tick_id.strip(), connectors, namespaces


def _recover_from_recorded_data(
    ledger: Ledger,
    wanted: list[str],
    namespaces: dict[str, str],
) -> list[plan.Instance]:
    """What connectors recorded about themselves, for the ones nothing else names.

    A connector that ever moved a row wrote the identity it ran under into that
    row, and those rows outlive its Secret. Read only where a Secret cannot
    answer — and never fatal: an unreadable relation costs this connector its
    recovery, not the tick its record.
    """
    recovered: list[plan.Instance] = []
    for connector in wanted:
        namespace = namespaces.get(connector, "")
        if not namespace:
            continue
        try:
            found = ledger.recorded_instance(namespace)
        except LedgerError as error:
            _LOG.warning(f"cannot read what {connector} recorded about itself: {error}")
            continue
        if found is None:
            continue
        tenant_id, source_id = found
        recovered.append(plan.Instance(connector, tenant_id, source_id))
    return recovered


def _adopt_unidentified_rows(
    ledger: Ledger,
    managed: list[plan.Instance],
    namespaces: dict[str, str],
) -> None:
    """Hand history recorded before the ledger carried identity to its instance.

    Never fatal, and never a reason not to record this tick: it corrects the
    past, and stopping the page's clock to do so would cost more than it fixes.
    A connector it cannot resolve keeps its empty identity and says why.
    """
    try:
        unidentified = ledger.unidentified_connectors()
    except LedgerError as error:
        _LOG.warning(f"cannot look for rows recorded without an identity: {error}")
        return
    if not unidentified:
        return

    entries = [heal.Unidentified(connector, has_syncs) for connector, has_syncs in unidentified]
    named_by_secret = {instance.connector for instance in managed}
    recovered = _recover_from_recorded_data(
        ledger,
        [entry.connector for entry in entries if entry.connector not in named_by_secret],
        namespaces,
    )

    adoption = heal.plan_adoption(entries, managed, recovered)
    if adoption.unresolved:
        # One line, not one per connector: an unresolvable name stays
        # unresolvable until its rows age out, so a line each would repeat every
        # tick for as long as the ledger retains them.
        _LOG.warning(
            "history left without an identity: "
            + "; ".join(f"{u.connector} ({u.reason})" for u in adoption.unresolved)
        )
    for claim in adoption.claimed:
        instance = claim.instance
        try:
            ledger.adopt_identity(instance)
        except LedgerError as error:
            _LOG.warning(f"cannot give {instance.connector} history its identity: {error}")
            continue
        _LOG.info(
            f"gave {instance.connector} history recorded before the identity "
            f"existed its own ({instance.tenant_id}/{instance.source_id}), "
            f"from {claim.basis}"
        )


def run(stream: Any) -> int:
    try:
        tick_id, connectors, namespaces = _read_work(stream)
    except UnreadableWork as error:
        _LOG.error(f"cannot read this tick's work: {error}")
        return 1

    # INVARIANT: an empty configured set is indistinguishable from "everything
    # was removed", so a tick with nothing to record records nothing at all —
    # not an empty snapshot, and no seal to make one readable.
    if not connectors:
        _LOG.warning("no connectors resolved; recording nothing rather than an empty set")
        return 1

    try:
        ledger = Ledger.from_env()
        mover = Mover.from_env()
    except (LedgerError, MoverError) as error:
        _LOG.error(f"cannot reach the inputs: {error}")
        return 1

    # Two different sets, deliberately. The connection map turns a job into a
    # connector, so it holds only connectors the mover has a connection for; a
    # job on a connection absent from it is skipped rather than guessed at. The
    # configured set is every connector the controller manages, whether or not
    # the mover has caught up — a connector awaiting its first connection is
    # configured and has never synced, which is a state the page must be able to
    # show.
    by_connection = {c.connection_id: c.instance for c in connectors if c.connection_id}
    configured = [c.instance for c in connectors]

    _adopt_unidentified_rows(ledger, configured, namespaces)

    read_failed = False
    incomplete = False
    written = 0
    try:
        watermark = ledger.watermark()
        closed = ledger.closed_job_ids(watermark)
        entries, truncated = mover.sync_jobs(plan.as_listing_stamp(watermark))

        # INVARIANT: an ignored filter is a failed read, not a noisy one. The
        # mover answers 200 and drops a parameter it does not recognise, so the
        # listing restarts at the beginning of history — and neither of the two
        # things that follow is survivable. Every terminal job below the
        # watermark would be recorded again every tick, because the closed-job
        # read is bounded by that same watermark and so cannot filter them out.
        # And a capped pass would stop short of current jobs while still
        # sealing, leaving the page dated as freshly checked on stale facts.
        unfiltered = plan.unfiltered_count(entries, watermark)
        if unfiltered:
            raise UnfilteredListing(
                f"{unfiltered} entry(ies) came back older than the watermark "
                "the listing was handed; it is not filtering on it"
            )
        if truncated:
            incomplete = True
            _LOG.warning(
                "history deeper than one tick may read; recorded what was "
                "reached and will continue from that edge next tick"
            )

        planned = plan.plan_syncs(entries, by_connection, tick_id, closed)
        for refusal in planned.skipped:
            _LOG.info(f"skipped job {refusal.job_id or '<unnamed>'}: {refusal.reason}")

        written += ledger.insert(planned.rows)

        # The read start is floored, so a job open longer than that floor will
        # never be asked about again. Say so, rather than leaving its last
        # provisional word standing as the page's answer.
        stranded = plan.plan_abandoned(ledger.abandoned_jobs(watermark), tick_id)
        if stranded:
            _LOG.warning(
                f"{len(stranded)} job(s) have fallen below the read start; recording their state as unreadable"
            )
            written += ledger.insert(stranded)
    # Every failure, not only the two typed ones: an unanticipated shape in the
    # listing would otherwise escape and skip the seal by accident rather than by
    # decision, which is the same outcome reached without the reasoning.
    except Exception as error:  # noqa: BLE001
        read_failed = True
        _LOG.error(f"could not read this tick's syncs: {error!r}")

    # INVARIANT: the seal is what dates the page — the read surface reports the
    # newest sealed tick as "when the mover was last read". A tick that never
    # read the mover must not seal, or an install whose mover is unreachable
    # keeps reporting that it was just checked, and the page can never say
    # recording has stopped.
    if read_failed:
        _LOG.error("the mover was not read; leaving this tick unsealed")
        return 1

    try:
        written += ledger.insert(plan.plan_snapshot(configured, tick_id))
        written += ledger.insert([plan.plan_seal(tick_id)])
    except LedgerError as error:
        _LOG.error(f"cannot seal this tick: {error}")
        return 1

    _LOG.info(
        f"tick {tick_id}: {written} rows across {len(configured)} connector instances"
    )
    return 1 if incomplete else 0


if __name__ == "__main__":
    run_id = os.environ.get("RECONCILE_RUN_ID", "").strip()
    insight_logging.configure("sweep", **({"run_id": run_id} if run_id else {}))
    sys.exit(run(sys.stdin))
