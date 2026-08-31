"""How the listing read decides it has read enough.

Two decisions, both easy to get subtly wrong and neither visible from the rows
that land: whether a short page means the end of the listing, and whether the
read ran out of time. Getting either wrong makes an incomplete tick report
itself complete, which is the one thing that stops FR-12 from ever firing.

Run: python3 -m unittest discover -s src/ingestion/reconcile-connectors/tests
"""

from __future__ import annotations

import sys
import unittest
import urllib.parse
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python"))

from sweep import mover as mover_module
from sweep.mover import Mover


class _Listing(Mover):
    """A mover whose pages are supplied rather than fetched.

    Overrides `_page`, the one seam between the read's decisions and its
    transport, so the decisions can be driven without a server.
    """

    def __init__(self, pages: list[tuple[int, int]]) -> None:
        super().__init__("http://mover.invalid", "token")
        #: (usable entries, entries the server served) per page.
        self._pages = pages
        self.requested: list[int] = []

    def _page(self, offset: int, updated_at_start: str | None):
        self.requested.append(offset)
        index = len(self.requested) - 1
        usable, served = self._pages[index] if index < len(self._pages) else (0, 0)
        return [{"jobId": offset + n} for n in range(usable)], served


class AShortPageEndsTheRead(unittest.TestCase):
    def test_a_page_shorter_than_the_size_is_the_end_of_the_listing(self) -> None:
        listing = _Listing([(mover_module.PAGE_SIZE, mover_module.PAGE_SIZE), (5, 5)])
        entries, truncated = listing.sync_jobs(None)

        self.assertEqual(len(entries), mover_module.PAGE_SIZE + 5)
        self.assertFalse(truncated)
        self.assertEqual(len(listing.requested), 2)

    def test_a_full_page_carrying_an_unusable_entry_is_not_a_short_page(self) -> None:
        """The end-of-listing test must read what the SERVER served.

        Measuring the filtered length instead makes a full page with one
        unusable element look like the end of the listing: the read stops, the
        tick reports itself complete, and the jobs above it wait for a tick that
        thinks it has nothing to do.
        """
        full = mover_module.PAGE_SIZE
        listing = _Listing([(full - 1, full), (3, 3)])
        entries, truncated = listing.sync_jobs(None)

        self.assertEqual(
            len(listing.requested), 2, "the read must continue past the filtered page"
        )
        self.assertEqual(len(entries), full - 1 + 3)
        self.assertFalse(truncated)


class TheReadHasATimeBudget(unittest.TestCase):
    def test_an_exhausted_budget_stops_the_read_and_says_so(self) -> None:
        """The page cap alone bounds nothing in time.

        Fifty pages at the per-request timeout outlives the tick's own deadline,
        and a tick killed part-way never reaches its summary — so the run reads
        as aborted and the next one is skipped.
        """
        full = mover_module.PAGE_SIZE
        listing = _Listing([(full, full)] * 4)
        original = mover_module.BUDGET_SECS
        mover_module.BUDGET_SECS = 0
        try:
            entries, truncated = listing.sync_jobs(None)
        finally:
            mover_module.BUDGET_SECS = original

        self.assertTrue(truncated, "a read that ran out of time is not a complete read")
        self.assertEqual(len(listing.requested), 1, "it stops rather than finishing")
        self.assertEqual(len(entries), full, "and keeps what it did read")

    def test_the_budget_leaves_room_under_the_ticks_own_deadline(self) -> None:
        # The workflow pod's deadline is 1800s; the sweep is its last layer and
        # still has the ledger writes and the seal to do afterwards.
        self.assertLess(mover_module.BUDGET_SECS, 900)

    def test_the_page_cap_is_a_backstop_not_the_budget(self) -> None:
        full = mover_module.PAGE_SIZE
        listing = _Listing([(full, full)] * (mover_module.MAX_PAGES + 5))
        _, truncated = listing.sync_jobs(None)

        self.assertTrue(truncated)
        self.assertEqual(len(listing.requested), mover_module.MAX_PAGES)


if __name__ == "__main__":
    unittest.main()


class TheRequestNamesTheFieldItReadsBack(unittest.TestCase):
    """The sort key, the filter and the field read off an entry are one stamp.

    The mover answers 200 and ignores a query parameter it does not recognise,
    so a filter under the wrong name does not fail — it stops filtering, the
    read restarts at the beginning of history, and a capped pass never reaches
    the newest jobs. Nothing downstream can tell that apart from a quiet page,
    which is why the request's own shape is asserted here.
    """

    def _query(self, updated_at_start: str | None) -> dict[str, str]:
        asked: list[str] = []

        class _Transport(Mover):
            def _get(self, path: str) -> dict:
                asked.append(path)
                return {"data": []}

        _Transport("http://mover.invalid", "token").sync_jobs(updated_at_start)
        self.assertEqual(len(asked), 1)
        query = urllib.parse.urlparse(asked[0]).query
        return dict(urllib.parse.parse_qsl(query))

    def test_the_listing_is_ordered_by_the_stamp_the_planner_records(self) -> None:
        self.assertEqual(self._query(None)["orderBy"], "updatedAt|ASC")

    def test_the_watermark_is_sent_as_the_update_filter(self) -> None:
        query = self._query("2026-08-27T08:02:52Z")

        self.assertEqual(query["updatedAtStart"], "2026-08-27T08:02:52Z")
        self.assertNotIn("createdAtStart", query)

    def test_a_first_sweep_sends_no_filter_at_all(self) -> None:
        """None is "read the whole retained history", not "read from now"."""
        query = self._query(None)

        self.assertNotIn("updatedAtStart", query)
        self.assertNotIn("createdAtStart", query)
