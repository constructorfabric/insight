"""Opt-in end-to-end sweep: a stub mover, a real ClickHouse, real rows.

Pure tests cover what the planner decides; this covers what actually lands. It
exists because an empty table hides everything that matters here — a column the
insert omits, a NULL the reader cannot take, a frontier that stops a job from
ever being re-read. Every one of those passes against zero rows.

Skipped unless a server is offered, so CI stays dependency-free:

    docker run -d --rm --name ch -p 38310:8123 \\
        -e CLICKHOUSE_USER=insight -e CLICKHOUSE_PASSWORD=insight \\
        clickhouse/clickhouse-server:25.7.5
    src/ingestion/scripts/lib/ch-exec.sh < the migration, then
    SWEEP_TEST_CH_URL=http://localhost:38310 \\
    SWEEP_TEST_CH_USER=insight SWEEP_TEST_CH_PASSWORD=insight \\
        python3 -m unittest discover -s src/ingestion/reconcile-connectors/tests

Run: python3 -m unittest discover -s src/ingestion/reconcile-connectors/tests
"""

from __future__ import annotations

import io
import json
import os
import sys
import threading
import unittest
import urllib.parse
import urllib.request
from datetime import UTC, datetime, timedelta
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
from typing import Self

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python"))

CH_URL = os.environ.get("SWEEP_TEST_CH_URL")
CH_USER = os.environ.get("SWEEP_TEST_CH_USER", "default")  # RULE-DEFAULTS-OK: local throwaway server's own default user
CH_PASSWORD = os.environ.get("SWEEP_TEST_CH_PASSWORD", "")  # RULE-DEFAULTS-OK: that user has no password

TABLE = "ingestion_history.sync_events"
CONNECTOR = "example-tracker"
CONNECTION = "connection-under-test"
STUB_PORT = 38399
TOKEN = "stub-token"

BASE_HOUR = 8
DAY = "2026-08-27"


def _stamp(hour_offset: int) -> str:
    """An ISO-8601 stamp, the shape the listing sends.

    Anchored to a fixed day and offset in hours, so a test can name a moment
    before the fixtures (a negative offset) without arithmetic at the call site.
    """
    moment = datetime(2026, 8, 27, BASE_HOUR, tzinfo=UTC) + timedelta(hours=hour_offset)
    return moment.strftime("%Y-%m-%dT%H:%M:%SZ")


def _ch_stamp(iso: str) -> str:
    """The ledger's own stamp form, for a direct INSERT."""
    return iso.replace("T", " ").replace("Z", "") + ".000"


def _parse_iso(stamp: str) -> datetime:
    """Refuses anything that is not ISO-8601, so a mis-sent watermark fails the
    test rather than quietly matching every entry."""
    return datetime.fromisoformat(stamp)


def _closed_job(index: int) -> dict:
    return {
        "jobId": 500 + index,
        "connectionId": CONNECTION,
        "status": "succeeded",
        "createdAt": _stamp(index),
        "startTime": _stamp(index),
        "duration": "PT3M10S",
        "rowsSynced": 100 * index,
    }


def _running_job() -> dict:
    """No start, no duration, no count — the mover has nothing to report yet."""
    return {
        "jobId": 999,
        "connectionId": CONNECTION,
        "status": "running",
        "createdAt": _stamp(8),
    }


def _finished_job() -> dict:
    return {
        "jobId": 999,
        "connectionId": CONNECTION,
        "status": "failed",
        "createdAt": _stamp(8),
        "startTime": _stamp(8),
        "duration": "PT5M",
        "rowsSynced": 0,
    }


class _StubMover:
    """The public job listing, serving only what the sweep asks it for.

    Asserts the two query parameters the sweep's correctness rests on: ascending
    creation order, so a capped read leaves only NEWER jobs unread, and the
    watermark filter, so a steady tick does not re-read the whole history.
    """

    def __init__(self, page_size: int = 100) -> None:
        self.oldest_first = [_closed_job(i) for i in range(7)] + [_running_job()]
        self.page_size = page_size
        self.requests: list[dict[str, str]] = []
        stub = self

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self) -> None:
                if self.headers.get("Authorization") != f"Bearer {TOKEN}":
                    self.send_error(401)
                    return
                query = urllib.parse.parse_qs(urllib.parse.urlparse(self.path).query)
                flat = {key: value[0] for key, value in query.items()}
                stub.requests.append(flat)

                assert flat.get("orderBy") == "createdAt|ASC", flat
                assert flat.get("jobType") == "sync", flat

                entries = stub.oldest_first
                since = flat.get("createdAtStart")
                if since is not None:
                    # A real listing parses this. Comparing the two stamp forms
                    # as raw strings is what let a wrongly-formatted watermark
                    # look like a working filter.
                    edge = _parse_iso(since)
                    entries = [e for e in entries if _parse_iso(e["createdAt"]) >= edge]
                offset = int(flat.get("offset", "0"))
                window = entries[offset : offset + stub.page_size]

                payload = json.dumps({"data": window}).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)

            def log_message(self, *args: object) -> None:
                pass

        self._server = HTTPServer(("127.0.0.1", STUB_PORT), Handler)

    def __enter__(self) -> Self:
        threading.Thread(target=self._server.serve_forever, daemon=True).start()
        return self

    def __exit__(self, *exc: object) -> None:
        self._server.shutdown()
        self._server.server_close()

    def job_finishes(self) -> None:
        self.oldest_first[-1] = _finished_job()


def _query(sql: str) -> str:
    # SAFETY: urllib honours `file://`. The URL is an opt-in env var, so the
    # scheme is pinned rather than taken on trust.
    assert CH_URL and CH_URL.startswith(("http://", "https://")), CH_URL
    request = urllib.request.Request(f"{CH_URL}/", data=sql.encode(), method="POST")
    request.add_header("X-ClickHouse-User", CH_USER)
    request.add_header("X-ClickHouse-Key", CH_PASSWORD)
    # nosemgrep: python.lang.security.audit.dynamic-urllib-use-detected.dynamic-urllib-use-detected
    with urllib.request.urlopen(request, timeout=30) as response:
        return response.read().decode()


def _rows(sql: str) -> list[dict]:
    raw = _query(f"{sql} FORMAT JSONEachRow")
    return [json.loads(line) for line in raw.splitlines() if line.strip()]


def _count(sql: str) -> int:
    """`UInt64` crosses JSON as a string — ClickHouse quotes it so a JS reader
    cannot silently lose precision. Every numeric assertion here goes through
    this rather than comparing against a bare int and passing by accident."""
    value = _rows(sql)[0]["n"]
    return int(value)


@unittest.skipUnless(CH_URL, "set SWEEP_TEST_CH_URL to run")
class SweptRowsLandAndResolve(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        from sweep import __main__ as entry

        cls.entry = entry
        os.environ.update(
            {
                "AIRBYTE_URL": f"http://127.0.0.1:{STUB_PORT}",
                "AIRBYTE_TOKEN": TOKEN,
                "RECONCILE_DEST_CLICKHOUSE_PROTOCOL": (
                    "https" if CH_URL.startswith("https") else "http"
                ),
                "RECONCILE_DEST_CLICKHOUSE_HOST": CH_URL.split("//", 1)[1].split(":")[0],
                "RECONCILE_DEST_CLICKHOUSE_PORT": CH_URL.rsplit(":", 1)[1],
                "RECONCILE_DEST_CLICKHOUSE_USERNAME": CH_USER,
                "RECONCILE_DEST_CLICKHOUSE_PASSWORD": CH_PASSWORD,
            }
        )

    def setUp(self) -> None:
        _query(f"TRUNCATE TABLE {TABLE}")

    def _tick(self, tick_id: str) -> int:
        work = json.dumps(
            {
                "tick_id": tick_id,
                "connectors": [{"name": CONNECTOR, "connection_id": CONNECTION}],
            }
        )
        return self.entry.run(io.StringIO(work))

    def test_a_first_sweep_lands_the_whole_retained_history(self) -> None:
        with _StubMover() as mover:
            self.assertEqual(self._tick("tick-a"), 0)
            self.assertIsNone(
                mover.requests[0].get("createdAtStart"),
                "an empty ledger reads everything, unfiltered",
            )

        syncs = _rows(
            f"SELECT job_id FROM {TABLE} WHERE event = 'sync.completed' ORDER BY job_id"
        )
        self.assertEqual(len(syncs), 8, "seven closed jobs plus the running one")

    def test_a_later_tick_reads_from_the_watermark_not_from_the_start(self) -> None:
        with _StubMover() as mover:
            self._tick("tick-a")
            mover.requests.clear()
            self._tick("tick-b")

            since = mover.requests[0].get("createdAtStart")
            self.assertIsNotNone(since, "a populated ledger must filter")
            # Not merely "a stamp": one the listing can parse. The ledger's own
            # form differs by a space, and a listing handed that form filters on
            # nothing at all.
            self.assertEqual(_parse_iso(since).tzinfo is None, False, since)

    def test_the_watermark_does_not_step_over_an_unfinished_job(self) -> None:
        """It stands on the oldest OPEN job, so that job is read again.

        Standing it on the newest job recorded would leave the one most likely
        still running below the line, and the page would show it running for
        ever.
        """
        with _StubMover() as mover:
            self._tick("tick-a")
            mover.requests.clear()
            self._tick("tick-b")

        since = mover.requests[0]["createdAtStart"]
        self.assertLessEqual(
            _parse_iso(since),
            _parse_iso(_stamp(8)),
            "the still-running job must stay at or above the watermark",
        )

    def test_the_tick_seals_after_its_snapshot(self) -> None:
        with _StubMover():
            self._tick("tick-a")

        seal = _rows(
            f"SELECT tick_id, toString(ts) AS ts FROM {TABLE} "
            "WHERE event = 'sweep.completed'"
        )
        snapshot = _rows(
            f"SELECT toString(max(ts)) AS ts FROM {TABLE} "
            "WHERE event = 'connector.configured'"
        )
        self.assertEqual(len(seal), 1)
        self.assertGreaterEqual(
            seal[0]["ts"], snapshot[0]["ts"], "the seal must be written last"
        )

    def test_a_second_tick_adds_no_row_for_a_closed_job(self) -> None:
        with _StubMover():
            self._tick("tick-a")
            self._tick("tick-b")

        closed = _rows(
            f"SELECT job_id, count() AS seen FROM {TABLE} "
            "WHERE event = 'sync.completed' AND status = 'succeeded' "
            "GROUP BY job_id HAVING seen > 1"
        )
        self.assertEqual(closed, [], "a job with a final account is left alone")

    def test_a_running_job_is_re_recorded_once_it_ends(self) -> None:
        """The one behaviour every other test here would pass without."""
        with _StubMover() as mover:
            self._tick("tick-a")
            mover.job_finishes()
            self._tick("tick-b")

        resolved = _rows(
            f"SELECT status, records_reported FROM {TABLE} "
            "WHERE event = 'sync.completed' AND job_id = '999' "
            "ORDER BY ts DESC LIMIT 1"
        )
        self.assertEqual(resolved[0]["status"], "failed")
        self.assertEqual(
            int(resolved[0]["records_reported"]),
            0,
            "a reported zero is not an absence",
        )

    def test_the_summary_resolves_to_the_newest_row_of_the_newest_job(self) -> None:
        with _StubMover() as mover:
            self._tick("tick-a")
            mover.job_finishes()
            self._tick("tick-b")

        summary = _rows(
            "SELECT connector, job_id, status FROM ("
            f"  SELECT connector, job_id, status, job_created_at FROM {TABLE}"
            "   WHERE event = 'sync.completed'"
            "   ORDER BY job_id, ts DESC LIMIT 1 BY job_id"
            ") ORDER BY connector, job_created_at DESC, job_id DESC LIMIT 1 BY connector"
        )
        self.assertEqual(len(summary), 1)
        self.assertEqual(summary[0]["job_id"], "999")
        self.assertEqual(summary[0]["status"], "failed")

    def test_an_unfinished_job_stores_no_start_and_no_duration(self) -> None:
        with _StubMover():
            self._tick("tick-a")

        running = _rows(
            f"SELECT started_at, duration_ms, records_reported FROM {TABLE} "
            "WHERE event = 'sync.completed' AND job_id = '999'"
        )
        self.assertIsNone(running[0]["started_at"])
        self.assertIsNone(running[0]["duration_ms"])
        self.assertIsNone(running[0]["records_reported"])

    def test_no_connectors_records_nothing_at_all(self) -> None:
        """An empty configured set is indistinguishable from "all removed"."""
        with _StubMover():
            code = self.entry.run(
                io.StringIO(json.dumps({"tick_id": "tick-z", "connectors": []}))
            )
        self.assertEqual(code, 1)
        self.assertEqual(_count(f"SELECT count() AS n FROM {TABLE}"), 0)

    def test_an_unreachable_mover_records_nothing_and_does_not_seal(self) -> None:
        """The seal dates the page, so an unread tick must not place one.

        Sealing anyway would keep `checked_at` advancing on an install whose
        mover is unreachable — the page would report that it was just checked
        for ever, and could never say recording had stopped.
        """
        # No stub server running: the listing call fails.
        code = self._tick("tick-y")
        self.assertEqual(code, 1, "the caller learns the tick was incomplete")
        for event in ("sync.completed", "connector.configured", "sweep.completed"):
            self.assertEqual(
                _count(
                    f"SELECT count() AS n FROM {TABLE} WHERE event = '{event}'"
                ),
                0,
                f"an unread tick must write no {event} row",
            )

    def test_a_connector_awaiting_its_first_connection_is_still_configured(self) -> None:
        """Configured is the first thing the page answers, and it does not
        depend on whether the mover has caught up yet."""
        work = json.dumps(
            {
                "tick_id": "tick-w",
                "connectors": [
                    {"name": CONNECTOR, "connection_id": CONNECTION},
                    {"name": "awaiting-connection"},
                ],
            }
        )
        with _StubMover():
            self.assertEqual(self.entry.run(io.StringIO(work)), 0)

        configured = {
            row["connector"]
            for row in _rows(
                f"SELECT connector FROM {TABLE} WHERE event = 'connector.configured'"
            )
        }
        self.assertEqual(configured, {CONNECTOR, "awaiting-connection"})

    def test_a_job_that_fell_below_the_read_start_stops_reading_as_running(self) -> None:
        """The floor's own cost, paid honestly.

        A job open longer than the floor will never be asked about again, so its
        last provisional word would otherwise stand as the page's answer for
        ever — the page reporting a sync as running long after it stopped being
        visible. The sweep records what is true instead: its state can no longer
        be read.
        """
        stale = _stamp(-24 * 60)
        _query(
            f"INSERT INTO {TABLE} (ts, tick_id, job_id, connector, event, status, "
            "started_at, job_created_at, duration_ms, records_reported) VALUES "
            f"(now64(3), 'old', 'stranded', '{CONNECTOR}', 'sync.completed', 'running', "
            f"NULL, toDateTime64('{_ch_stamp(stale)}', 3, 'UTC'), NULL, NULL)"
        )
        with _StubMover():
            # Two ticks: the read start is resolved before this tick's rows
            # land, so the floor only moves past the stranded job once newer
            # jobs are recorded. One tick of delay, and it settles itself.
            self._tick("tick-a")
            self._tick("tick-b")

        resolved = _rows(
            f"SELECT status FROM {TABLE} WHERE event = 'sync.completed' "
            "AND job_id = 'stranded' ORDER BY ts DESC LIMIT 1"
        )
        self.assertEqual(
            resolved[0]["status"],
            "unknown",
            "a job the sweep can no longer see must not keep reading as running",
        )

    def test_a_stranded_job_is_marked_once_and_not_every_tick(self) -> None:
        """`unknown` is what stops the job being named again.

        The marker is a row like any other, so without excluding an
        already-marked job from the search the sweep would write one per tick for
        ever. `unknown` earns its place in that exclusion here and nowhere else:
        it stays NON-terminal for coverage, because an unreadable status is one
        to keep re-reading while the job is still in the window.
        """
        stale = _stamp(-24 * 60)
        _query(
            f"INSERT INTO {TABLE} (ts, tick_id, job_id, connector, event, status, "
            "started_at, job_created_at, duration_ms, records_reported) VALUES "
            f"(now64(3), 'old', 'stranded', '{CONNECTOR}', 'sync.completed', 'running', "
            f"NULL, toDateTime64('{_ch_stamp(stale)}', 3, 'UTC'), NULL, NULL)"
        )
        with _StubMover():
            self._tick("tick-a")
            self._tick("tick-b")
            self._tick("tick-c")

        markers = _count(
            f"SELECT count() AS n FROM {TABLE} WHERE job_id = 'stranded' "
            "AND status = 'unknown'"
        )
        self.assertEqual(markers, 1, "the marker must settle, not repeat")

    def test_a_stuck_open_job_does_not_pin_the_watermark_for_ever(self) -> None:
        """One job that can never close would otherwise drag the read start back
        to its own creation time on every tick, until more jobs than a tick may
        read sit above it and the newest syncs stop being read at all."""
        stale = _stamp(-24 * 60)  # a month back, in the same shape the mover sends
        _query(
            f"INSERT INTO {TABLE} (ts, tick_id, job_id, connector, event, status, "
            "started_at, job_created_at, duration_ms, records_reported) VALUES "
            f"(now64(3), 'old', 'stuck', '{CONNECTOR}', 'sync.completed', 'running', "
            f"NULL, toDateTime64('{_ch_stamp(stale)}', 3, 'UTC'), NULL, NULL)"
        )
        with _StubMover() as mover:
            self._tick("tick-a")
            mover.requests.clear()
            self._tick("tick-b")

        since = _parse_iso(mover.requests[0]["createdAtStart"])
        self.assertGreater(
            since,
            _parse_iso(stale),
            "the stuck job must not hold the read start at its own creation time",
        )


if __name__ == "__main__":
    unittest.main()
