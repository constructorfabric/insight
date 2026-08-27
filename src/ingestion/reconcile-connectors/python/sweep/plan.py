"""Decide which rows a tick writes. Pure functions over values — no I/O.

The shell gathers, the planner decides, and the planner is what the tests
exercise. Its rules are this change's densest logic: which jobs to cover, which
already-recorded rows close a job, and which jobs cannot be placed at all.

Every field read here comes from the mover's own listing entry, whose shape is
flat: `jobId`, `connectionId`, `status`, `lastUpdatedAt`, `startTime`,
`duration`, `rowsSynced`. The stamps are ISO-8601 strings and the duration is an
ISO-8601 duration, so both are parsed rather than cast.

INVARIANT: the entry carries no creation time. The listing accepts a creation
filter but never reports one, so reading a job's moment from anything other
than its last-update stamp refuses every entry — indistinguishable, from the
page, from a mover that has run no syncs at all.
"""

from __future__ import annotations

import math
import re
from collections.abc import Iterable, Mapping
from datetime import UTC, datetime
from typing import Any, NamedTuple

from . import status as vocab

SYNC_COMPLETED = "sync.completed"
CONNECTOR_CONFIGURED = "connector.configured"
SWEEP_COMPLETED = "sweep.completed"

#: ClickHouse reads `DateTime64(3, 'UTC')` from this shape.
_MOMENT_FORMAT = "%Y-%m-%d %H:%M:%S.%f"

#: What `DateTime64(3)` can hold. A stamp outside it is not rejected on insert —
#: ClickHouse CLAMPS it silently, so a year 1000 stamp lands as 1900 and a year
#: 9999 stamp as 2299. Either would make the page state a moment nobody
#: recorded, and a clamped-low stamp would additionally pin the sweep's own
#: watermark to 1900 and re-read the whole retained history every tick.
_EARLIEST_MOMENT = datetime(1900, 1, 1, tzinfo=UTC)
_LATEST_MOMENT = datetime(2299, 12, 31, tzinfo=UTC)

#: `UInt64`. A larger value is not rejected either — it wraps, and the page
#: would label the wrapped number as what the mover reported.
_MAX_UINT64 = 2**64 - 1

#: Columns inapplicable to a row class carry an empty string rather than NULL:
#: they are `LowCardinality(String)`, and every read filters by `event` before
#: touching them.
_ABSENT = ""

#: `PnDTnHnMnS`, any component optional, seconds possibly fractional.
_ISO_DURATION = re.compile(
    r"^P(?:(?P<days>\d+(?:\.\d+)?)D)?"
    r"(?:T(?:(?P<hours>\d+(?:\.\d+)?)H)?"
    r"(?:(?P<minutes>\d+(?:\.\d+)?)M)?"
    r"(?:(?P<seconds>\d+(?:\.\d+)?)S)?)?$"
)

_SECONDS_PER = {"days": 86_400.0, "hours": 3_600.0, "minutes": 60.0, "seconds": 1.0}


class Skipped(NamedTuple):
    """A job the planner refused, and why. Logged, never written."""

    job_id: str
    reason: str


class Plan(NamedTuple):
    rows: list[dict[str, Any]]
    skipped: list[Skipped]


def moment(stamp: object) -> str | None:
    """Format the mover's ISO-8601 stamp for ClickHouse; None when unusable.

    A stamp that will not parse is not a moment. Substituting the epoch would
    make the page state a time nobody recorded, and would put the job at the
    bottom of every ordering the ledger does along this axis.
    """
    if not isinstance(stamp, str) or not stamp.strip():
        return None
    try:
        # `fromisoformat` takes the `Z` suffix directly from 3.11 on, which is
        # the shape the listing actually sends.
        parsed = datetime.fromisoformat(stamp.strip())
    except ValueError:
        return None
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=UTC)
    parsed = parsed.astimezone(UTC)
    if not _EARLIEST_MOMENT <= parsed <= _LATEST_MOMENT:
        return None
    return parsed.strftime(_MOMENT_FORMAT)[:-3]


def entry_moment(entry: Mapping[str, Any]) -> str | None:
    """One listing entry's last-update moment, in the ledger's own form.

    The single reader of that field: the watermark is compared against it, the
    listing is filtered by it and the planner records it, and the three must
    never disagree about which of an entry's stamps places it.
    """
    return moment(entry.get("lastUpdatedAt"))


def as_listing_stamp(recorded: str | None) -> str | None:
    """The ledger's stamp in the form the mover's listing filter understands.

    The two formats differ by a space and a suffix, and sending the wrong one is
    silent: a listing handed `2026-08-27 16:00:00.000` does not filter on it, so
    the sweep would re-read the whole history every tick and think it had
    filtered. Converting in one named place is the only way that stays visible.

    `unfiltered_count` is the other half of that guard — this one shapes the
    request, and that one checks the answer.
    """
    if recorded is None:
        return None
    return recorded.strip().replace(" ", "T") + "Z"


def unfiltered_count(entries: Iterable[Mapping[str, Any]], watermark: str | None) -> int:
    """How many entries came back older than the watermark the listing was handed.

    The mover ignores a query parameter it does not recognise instead of
    refusing it, so a filter a later release renames stops filtering silently:
    the read restarts at the beginning of history, a capped pass never reaches
    the newest jobs, and the page stops advancing with nothing to say why. Any
    entry older than what was asked for proves the filter did not apply.

    Compared as strings deliberately — both sides are the ledger's own
    fixed-width form, where lexicographic order is chronological order.
    """
    if watermark is None:
        return 0
    older = 0
    for entry in entries:
        placed = entry_moment(entry)
        if placed is not None and placed < watermark:
            older += 1
    return older


def duration_ms(duration: object) -> int | None:
    """Milliseconds from the mover's ISO-8601 duration (`PT1M37S`).

    None for anything that does not parse. Reporting a duration shorter than the
    truth because one component went unread would be worse than reporting none:
    the page would state a measurement nobody made.
    """
    if isinstance(duration, bool):
        return None
    if isinstance(duration, (int, float)):
        # A bare number is seconds, which is what the listing's own duration
        # spells out; anything else would be this reading a unit into a field
        # the mover documents as ISO-8601.
        return _bounded(float(duration) * 1000) if duration >= 0 else None
    if not isinstance(duration, str) or not duration.strip():
        return None

    matched = _ISO_DURATION.match(duration.strip().upper())
    if matched is None:
        return None
    parts = matched.groupdict()
    if all(value is None for value in parts.values()):
        return None
    total = sum(
        float(parts[name]) * seconds
        for name, seconds in _SECONDS_PER.items()
        if parts[name] is not None
    )
    return _bounded(total * 1000)


def _bounded(value: float) -> int | None:
    """A magnitude the column can hold, or None.

    `UInt64` wraps rather than rejects, so an out-of-range count would be stored
    as a different number and labelled as the mover's own.

    Finiteness is checked HERE rather than at each call site: `int(inf)` raises,
    and an exception on one malformed field costs the whole tick its seal — a
    listing that sends one nonsense number would stop the page's clock.
    """
    if not math.isfinite(value):
        return None
    return int(value) if 0 <= value <= _MAX_UINT64 else None


def records_reported(entry: Mapping[str, Any]) -> int | None:
    """What the mover states it moved. None where it reported no count at all.

    None rather than zero: a sync that moved nothing and a sync nobody counted
    are different answers, and the page prints them differently.
    """
    value = entry.get("rowsSynced")
    if isinstance(value, bool):
        return None
    if isinstance(value, str):
        # A count the listing sent as text is still a count.
        try:
            value = int(value)
        except ValueError:
            return None
    if not isinstance(value, (int, float)):
        return None
    return _bounded(value) if value >= 0 else None


def sync_row(
    entry: Mapping[str, Any], connectors: Mapping[str, str], tick_id: str
) -> dict[str, Any] | Skipped:
    """One ledger row for one listing entry, or the reason it cannot be recorded."""
    raw_id = entry.get("jobId")
    if raw_id is None or isinstance(raw_id, bool):
        return Skipped("", "listing entry carries no job identity")
    job_id = str(raw_id).strip()
    if not job_id:
        # Several such jobs would share one key and replace each other during
        # resolution, so the page would answer with whichever landed last.
        return Skipped("", "listing entry carries an empty job identity")

    connection = entry.get("connectionId")
    connector = connectors.get(str(connection)) if connection is not None else None
    if not connector:
        # A job on a connection this install does not manage: another tenant's,
        # or one left behind by a connector since removed. Recording it under a
        # guessed name would put syncs on the wrong row.
        return Skipped(job_id, "job belongs to no managed connection")

    # SAFETY: the summary resolves the newest sync per connector along this
    # column, and a row without it can never win that comparison. Recording it
    # anyway would hide the connector rather than the row.
    updated = entry_moment(entry)
    if updated is None:
        return Skipped(job_id, "job carries no readable update time")

    return {
        "tick_id": tick_id,
        "job_id": job_id,
        "connector": connector,
        "event": SYNC_COMPLETED,
        "status": vocab.normalise(entry.get("status")),
        "started_at": moment(entry.get("startTime")),
        "job_updated_at": updated,
        "duration_ms": duration_ms(entry.get("duration")),
        "records_reported": records_reported(entry),
    }


def plan_syncs(
    entries: Iterable[Mapping[str, Any]],
    connectors: Mapping[str, str],
    tick_id: str,
    closed_job_ids: frozenset[str],
) -> Plan:
    """Rows for the jobs this tick still has something to say about.

    A job the ledger already holds with a terminal status is left alone: its
    account cannot change, so re-recording it would only add a row resolving to
    the same answer.
    """
    rows: list[dict[str, Any]] = []
    skipped: list[Skipped] = []
    for entry in entries:
        planned = sync_row(entry, connectors, tick_id)
        if isinstance(planned, Skipped):
            skipped.append(planned)
            continue
        if planned["job_id"] in closed_job_ids:
            continue
        rows.append(planned)
    return Plan(rows, skipped)


def plan_abandoned(
    jobs: Iterable[Mapping[str, str]], tick_id: str
) -> list[dict[str, Any]]:
    """Mark jobs the sweep can no longer read as unreadable.

    A job that has fallen below the read start will not be asked about again, so
    its last provisional word would otherwise stand as the page's answer for
    ever — the page reporting a sync as running long after it stopped being
    visible. `unknown` is the honest replacement: not a failure, not a success,
    a state that can no longer be read.
    """
    rows = []
    for job in jobs:
        job_id = str(job.get("job_id", ""))
        connector = str(job.get("connector", ""))
        updated = str(job.get("updated", ""))
        if not job_id or not connector or not updated:
            continue
        rows.append(
            {
                "tick_id": tick_id,
                "job_id": job_id,
                "connector": connector,
                "event": SYNC_COMPLETED,
                "status": vocab.UNKNOWN,
                "started_at": None,
                "job_updated_at": updated,
                "duration_ms": None,
                "records_reported": None,
            }
        )
    return rows


def _bare_row(tick_id: str, event: str, connector: str) -> dict[str, Any]:
    return {
        "tick_id": tick_id,
        "job_id": _ABSENT,
        "connector": connector,
        "event": event,
        "status": _ABSENT,
        "started_at": None,
        "job_updated_at": None,
        "duration_ms": None,
        "records_reported": None,
    }


def plan_snapshot(connectors: Iterable[str], tick_id: str) -> list[dict[str, Any]]:
    """One row per connector the controller manages this tick."""
    return [
        _bare_row(tick_id, CONNECTOR_CONFIGURED, connector)
        for connector in sorted(set(connectors))
    ]


def plan_seal(tick_id: str) -> dict[str, Any]:
    """The marker written last, which makes the tick's snapshot readable.

    INVARIANT: nothing else may be written under this tick id afterwards. A read
    keys the configured set on the newest sealed tick, so a row arriving after
    its seal would join a snapshot already being read as complete.
    """
    return _bare_row(tick_id, SWEEP_COMPLETED, _ABSENT)
