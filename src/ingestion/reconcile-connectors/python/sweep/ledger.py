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

#: `LowCardinality(String)` and quoted into SQL below, so the vocabulary is
#: rendered rather than bound. Everything here is a literal from this module.
_TERMINAL_LIST = ", ".join(f"'{word}'" for word in sorted(vocab.TERMINAL_STATUSES))


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
        ended — the page would show a sync running forever.

        So the watermark is the oldest job still open, and only when nothing is
        open does it move up to the newest job recorded. Every unfinished job
        therefore stays at or above the line and is re-read until it closes,
        which costs one duplicate row per tick while it runs. No assumption
        about how many jobs the mover runs at once is needed for that to hold.

        None means nothing is recorded yet, and the first sweep reads the whole
        retained history.
        """
        rows = self._select(
            "SELECT toString(coalesce(min(if(open, job_created_at, NULL)), "
            "max(job_created_at))) AS watermark FROM ("
            "  SELECT job_created_at, "
            f"    status NOT IN ({_TERMINAL_LIST}) AS open "
            f"  FROM {TABLE} WHERE event = '{SYNC_COMPLETED}' "
            "    AND job_created_at IS NOT NULL "
            "  ORDER BY job_id, ts DESC LIMIT 1 BY job_id)"
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
            window = f" AND job_created_at >= {_quote(since)}"
        rows = self._select(
            f"SELECT DISTINCT job_id FROM {TABLE} "
            f"WHERE event = '{SYNC_COMPLETED}' "
            f"AND status IN ({_TERMINAL_LIST}){window}"
        )
        return frozenset(str(row["job_id"]) for row in rows)

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
