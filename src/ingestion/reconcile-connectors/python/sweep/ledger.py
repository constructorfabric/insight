"""Reading from and writing to the sync ledger over ClickHouse's HTTP interface.

Credentials come from the `RECONCILE_DEST_CLICKHOUSE_*` env the reconcile loop
already requires for the Bronze destination, so the sweep adds no chart value.
"""

from __future__ import annotations

import json
import os
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Iterable
from typing import Any

from . import status as vocab
from .plan import SYNC_COMPLETED

TABLE = "ingestion_history.sync_events"

_TIMEOUT_SECS = 30

#: How far behind the newest recorded job the watermark may be dragged by a job
#: that never closes. Long enough that no real sync is missed by it; short
#: enough that a stuck job costs one bounded re-read per tick rather than the
#: whole retention window.
LOOKBACK_DAYS = 7

#: `LowCardinality(String)` and quoted into SQL below, so the vocabulary is
#: rendered rather than bound. Everything here is a literal from this module.
_TERMINAL_LIST = ", ".join(f"'{word}'" for word in sorted(vocab.TERMINAL_STATUSES))

#: What stops a job counting as open for the read start.
#:
#: `UNKNOWN` joins the terminal words here and only here. It stays NON-terminal
#: for coverage, because an unreadable status is one to keep re-reading while the
#: job is still in the window. What this list adds is that an already-marked job
#: is neither searched for again — the marker would otherwise be written once a
#: tick for ever — nor allowed to drag the read start back to its own creation
#: time.
_OPEN_EXCLUDED = ", ".join(
    f"'{word}'" for word in sorted({*vocab.TERMINAL_STATUSES, vocab.UNKNOWN})
)


class LedgerError(RuntimeError):
    """Anything that stopped the sweep from reading or writing the ledger."""


class Ledger:
    def __init__(self, url: str, user: str, password: str) -> None:
        self._url = url.rstrip("/")
        self._user = user
        self._password = password

    @classmethod
    def from_env(cls, env: dict[str, str] | None = None) -> Ledger:
        source = os.environ if env is None else env
        missing = [
            name
            for name in (
                "RECONCILE_DEST_CLICKHOUSE_HOST",
                "RECONCILE_DEST_CLICKHOUSE_PORT",
                "RECONCILE_DEST_CLICKHOUSE_USERNAME",
                "RECONCILE_DEST_CLICKHOUSE_PASSWORD",
            )
            if not source.get(name)
        ]
        if missing:
            raise LedgerError(f"ClickHouse env incomplete: {', '.join(missing)}")
        proto_key = "RECONCILE_DEST_CLICKHOUSE_PROTOCOL"
        protocol = source.get(proto_key, "http")  # RULE-DEFAULTS-OK: chart-rendered constant, same fallback the Bronze destination config uses
        # SAFETY: urllib honours `file://`, so the scheme is pinned rather than
        # taken on trust from the environment.
        if protocol not in ("http", "https"):
            raise LedgerError(f"{proto_key} must be http or https, not {protocol!r}")
        host = source["RECONCILE_DEST_CLICKHOUSE_HOST"]
        port = source["RECONCILE_DEST_CLICKHOUSE_PORT"]
        return cls(
            f"{protocol}://{host}:{port}",
            source["RECONCILE_DEST_CLICKHOUSE_USERNAME"],
            source["RECONCILE_DEST_CLICKHOUSE_PASSWORD"],
        )

    def _post(self, body: bytes, query: str | None = None) -> str:
        url = self._url + "/"
        if query is not None:
            url += "?query=" + urllib.parse.quote(query)
        request = urllib.request.Request(url, data=body, method="POST")
        request.add_header("X-ClickHouse-User", self._user)
        request.add_header("X-ClickHouse-Key", self._password)
        try:
            # nosemgrep: python.lang.security.audit.dynamic-urllib-use-detected.dynamic-urllib-use-detected
            with urllib.request.urlopen(request, timeout=_TIMEOUT_SECS) as response:
                return response.read().decode()
        except urllib.error.HTTPError as error:
            detail = error.read().decode(errors="replace").strip()
            raise LedgerError(f"ClickHouse rejected the request: {detail}") from error
        except OSError as error:
            raise LedgerError(f"ClickHouse unreachable: {error}") from error

    def _select(self, sql: str) -> list[dict[str, Any]]:
        raw = self._post(f"{sql} FORMAT JSONEachRow".encode())
        return [json.loads(line) for line in raw.splitlines() if line.strip()]

    def watermark(self) -> str | None:
        """Where the next read of the mover's listing should start.

        Not the newest job recorded: that is the one most likely still running,
        and a watermark standing on it would never let a later tick see how it
        ended — the page would show a sync running for ever. So it stands on the
        oldest job still open, and only when nothing is open does it move up to
        the newest job recorded.

        INVARIANT: floored at `LOOKBACK_DAYS` behind the newest recorded job.
        Without a floor, one job that can never close pins the read start for
        ever — and three inputs produce one: a connection deleted while a job
        was running (the mover drops the job, so its last recorded word stays
        provisional), a status word a later mover release adds (stored as
        `unknown`, which is deliberately non-terminal), and an update stamp the
        column cannot hold. Once the start is pinned and more jobs than one tick
        may read sit above it, the newest syncs stop being read at all and the
        page freezes on stale data with only a log line to say so.

        None means nothing is recorded yet, and the first sweep reads the whole
        retained history.
        """
        rows = self._select(
            "SELECT toString(if(open_count > 0, "
            f"    greatest(oldest_open, newest - INTERVAL {LOOKBACK_DAYS} DAY), "
            "    newest)) AS watermark "
            "FROM (SELECT countIf(open) AS open_count, "
            "             min(if(open, job_updated_at, NULL)) AS oldest_open, "
            "             max(job_updated_at) AS newest "
            "      FROM (SELECT job_updated_at, "
            f"                   status NOT IN ({_OPEN_EXCLUDED}) AS open "
            f"            FROM {TABLE} WHERE event = '{SYNC_COMPLETED}' "
            "              AND job_updated_at IS NOT NULL "
            "            ORDER BY job_id, ts DESC LIMIT 1 BY job_id))"
        )
        if not rows:
            return None
        covered = rows[0].get("watermark")
        return covered if isinstance(covered, str) and covered else None

    def closed_job_ids(self, since: str | None) -> frozenset[str]:
        """Jobs the ledger already holds a terminal status for.

        Bounded by the same watermark the listing is: those are the only jobs
        this tick can be handed, and an unbounded read would grow with the
        table's whole retention.
        """
        window = ""
        if since is not None:
            window = f" AND job_updated_at >= {_quote(since)}"
        rows = self._select(
            f"SELECT DISTINCT job_id FROM {TABLE} "
            f"WHERE event = '{SYNC_COMPLETED}' "
            f"AND status IN ({_TERMINAL_LIST}){window}"
        )
        return frozenset(str(row["job_id"]) for row in rows)

    def abandoned_jobs(self, watermark: str | None) -> list[dict[str, str]]:
        """Jobs still recorded as open that have fallen below the read start.

        The read start is floored so one job that never closes cannot pin it for
        ever. The cost of that floor is these jobs: the mover will not be asked
        about them again, so their last recorded word — a provisional one — would
        stand as the page's answer indefinitely, and the page would report a sync
        as running years after it stopped being visible.

        Naming them lets the sweep record what is actually true: their state can
        no longer be read.
        """
        if watermark is None:
            return []
        return self._select(
            "SELECT job_id, connector, toString(job_updated_at) AS updated FROM ("
            "  SELECT job_id, connector, job_updated_at, status "
            f"  FROM {TABLE} WHERE event = '{SYNC_COMPLETED}' "
            "    AND job_updated_at IS NOT NULL "
            "  ORDER BY job_id, ts DESC LIMIT 1 BY job_id) "
            f"WHERE status NOT IN ({_OPEN_EXCLUDED}) "
            f"  AND job_updated_at < {_quote(watermark)}"
        )

    def insert(self, rows: Iterable[dict[str, Any]]) -> int:
        """Append rows. Returns how many were written."""
        payload = "\n".join(json.dumps(row) for row in rows)
        if not payload:
            return 0
        self._post(payload.encode(), query=f"INSERT INTO {TABLE} FORMAT JSONEachRow")
        return payload.count("\n") + 1


def _quote(value: str) -> str:
    """Single-quote a literal for ClickHouse.

    The HTTP interface takes no bind parameters on this path, and connector
    names come from descriptors on disk rather than from a request — but a name
    is still a string, and a string spliced into SQL unquoted is a habit worth
    not having.
    """
    escaped = value.replace("\\", "\\\\").replace("'", "\\'")
    return f"'{escaped}'"
