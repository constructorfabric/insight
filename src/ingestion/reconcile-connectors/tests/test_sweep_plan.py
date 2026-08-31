"""What a tick decides to write.

Run: python3 -m unittest discover -s src/ingestion/reconcile-connectors/tests
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python"))

from sweep import plan
from sweep import status as vocab

CONNECTOR = "example-tracker"
TICK = "tick-1"

CONNECTION = "connection-under-test"
CONNECTORS = {CONNECTION: CONNECTOR}

STARTED = "2026-08-27T08:00:30Z"
UPDATED = "2026-08-27T08:02:52Z"

#: Every key the listing serves, and nothing it does not. Kept as data because a
#: fixture that invents a field name is how a planner comes to read one the
#: mover never sends: every entry is then refused, and a page reporting every
#: connector as never synced cannot be told apart from a mover that ran nothing.
SERVED_KEYS = frozenset(
    {
        "jobId",
        "connectionId",
        "jobType",
        "status",
        "startTime",
        "lastUpdatedAt",
        "duration",
        "bytesSynced",
        "rowsSynced",
    }
)


def entry(**overrides: object) -> dict[str, object]:
    """One entry of the mover's job listing, in the shape it serves.

    Flat, ISO-8601 stamps, an ISO-8601 duration, and the record count on the
    entry itself — there is no nested job or attempts array on this listing, and
    no creation stamp: the listing accepts a creation filter but reports only
    when a job started and when it was last updated.
    """
    listing_entry = {
        "jobId": 8412,
        "connectionId": CONNECTION,
        "jobType": "sync",
        "status": "succeeded",
        "startTime": STARTED,
        "lastUpdatedAt": UPDATED,
        "duration": "PT2M22S",
        "bytesSynced": 3_100_000,
        "rowsSynced": 12_400,
    }
    listing_entry.update(overrides)
    return listing_entry


def row(**overrides: object) -> dict[str, object]:
    planned = plan.sync_row(entry(**overrides), CONNECTORS, TICK)
    assert not isinstance(planned, plan.Skipped), planned
    return planned


class StatusKeepsTheMoversWord(unittest.TestCase):
    def test_documented_statuses_are_stored_verbatim(self) -> None:
        for word in sorted(vocab.MOVER_STATUSES):
            with self.subTest(word=word):
                self.assertEqual(row(status=word)["status"], word)

    def test_an_undocumented_word_is_stored_as_unknown(self) -> None:
        for word in ("SUCCEEDED_PARTIALLY", "", None, 7, "  "):
            with self.subTest(word=word):
                self.assertEqual(
                    row(status=word)["status"],
                    vocab.UNKNOWN,
                    f"should not pass through: {word!r}",
                )

    def test_case_and_padding_do_not_produce_an_unknown(self) -> None:
        self.assertEqual(row(status=" Succeeded ")["status"], "succeeded")

    def test_unknown_does_not_close_a_job(self) -> None:
        """Coverage fails closed: a status we could not read is re-read."""
        self.assertFalse(vocab.is_terminal(vocab.UNKNOWN))

    def test_an_unfinished_status_does_not_close_a_job(self) -> None:
        for word in ("pending", "running"):
            with self.subTest(word=word):
                self.assertFalse(vocab.is_terminal(word))


class AJobMustBePlaceable(unittest.TestCase):
    def test_a_job_with_neither_stamp_readable_is_refused(self) -> None:
        """Both, because either one places the job. A case that blanks only the
        update stamp stops testing the refusal the moment a fallback exists."""
        for absent in (None, 0, -1, "yesterday", True):
            with self.subTest(absent=absent):
                planned = plan.sync_row(
                    entry(lastUpdatedAt=absent, startTime=absent), CONNECTORS, TICK
                )
                self.assertIsInstance(
                    planned, plan.Skipped, f"should refuse both stamps = {absent!r}"
                )

    def test_a_job_with_no_identity_is_refused(self) -> None:
        planned = plan.sync_row(entry(jobId=None), CONNECTORS, TICK)
        self.assertIsInstance(planned, plan.Skipped)

    def test_an_entry_with_no_job_is_refused(self) -> None:
        planned = plan.sync_row({"connectionId": CONNECTION}, CONNECTORS, TICK)
        self.assertIsInstance(planned, plan.Skipped)

    def test_a_refusal_carries_its_reason(self) -> None:
        planned = plan.sync_row(
            entry(lastUpdatedAt=None, startTime=None), CONNECTORS, TICK
        )
        self.assertIn("moment", planned.reason)


class AbsenceIsNotZero(unittest.TestCase):
    def test_a_job_not_started_has_no_start_and_no_duration(self) -> None:
        bare = {
            "jobId": 1,
            "connectionId": CONNECTION,
            "status": "pending",
            "lastUpdatedAt": UPDATED,
        }
        planned = plan.sync_row(bare, CONNECTORS, TICK)
        self.assertIsNone(planned["started_at"])
        self.assertIsNone(planned["duration_ms"])
        self.assertIsNone(planned["records_reported"])

    def test_a_reported_zero_survives_as_zero(self) -> None:
        """A sync that moved nothing is not a sync nobody counted."""
        self.assertEqual(row(rowsSynced=0)["records_reported"], 0)

    def test_no_reported_count_at_all_is_absent(self) -> None:
        for absent in (None, "", "many", True):
            with self.subTest(absent=absent):
                self.assertIsNone(row(rowsSynced=absent)["records_reported"])

    def test_a_negative_count_is_absent_rather_than_recorded(self) -> None:
        self.assertIsNone(row(rowsSynced=-1)["records_reported"])

    def test_a_non_finite_number_is_absent_rather_than_an_exception(self) -> None:
        """`int(inf)` raises, and one malformed field would cost the whole tick
        its seal — which stops the page's own clock."""
        for absent in (float("nan"), float("inf"), float("-inf")):
            with self.subTest(absent=absent):
                self.assertIsNone(row(duration=absent)["duration_ms"])
                self.assertIsNone(row(rowsSynced=absent)["records_reported"])

    def test_an_empty_job_identity_is_refused(self) -> None:
        """Several such jobs would share one key and replace each other during
        resolution, so the page would answer with whichever landed last."""
        for empty in ("", "   "):
            with self.subTest(empty=empty):
                planned = plan.sync_row(entry(jobId=empty), CONNECTORS, TICK)
                self.assertIsInstance(planned, plan.Skipped)

    def test_a_duration_that_will_not_parse_is_absent(self) -> None:
        """Reading only some components would report a span shorter than the
        truth, which is worse than reporting none."""
        for absent in ("1m37s", "P", "", None, "PT", "garbage"):
            with self.subTest(absent=absent):
                self.assertIsNone(row(duration=absent)["duration_ms"])


class WhatIsRead(unittest.TestCase):
    def test_the_stamps_are_read_as_iso_8601(self) -> None:
        planned = row()
        self.assertEqual(planned["job_updated_at"], "2026-08-27 08:02:52.000")
        self.assertEqual(planned["started_at"], "2026-08-27 08:00:30.000")

    def test_a_stamp_with_an_offset_is_normalised_to_utc(self) -> None:
        planned = row(lastUpdatedAt="2026-08-27T11:00:00+03:00")
        self.assertEqual(planned["job_updated_at"], "2026-08-27 08:00:00.000")

    def test_the_placing_stamp_is_the_last_update_not_the_start(self) -> None:
        """Two stamps arrive and only one places the job.

        Reading the other would still produce a row, so nothing but naming the
        expected value catches a planner that took `startTime` for the axis the
        watermark moves along.
        """
        self.assertEqual(row()["job_updated_at"], "2026-08-27 08:02:52.000")
        self.assertNotEqual(row()["job_updated_at"], row()["started_at"])

    def test_an_epoch_number_is_not_a_stamp(self) -> None:
        """The listing sends strings. A number here would be a different
        endpoint's shape, and guessing at it would date every job to 1970."""
        planned = plan.sync_row(
            entry(lastUpdatedAt=1_700_000_000, startTime=1_700_000_000),
            CONNECTORS,
            TICK,
        )
        self.assertIsInstance(planned, plan.Skipped)

    def test_the_duration_is_read_as_an_iso_8601_duration(self) -> None:
        cases = {
            "PT1M37S": 97_000,
            "PT2H3M4.5S": 7_384_500,
            "P1DT1S": 86_401_000,
            "PT0.25S": 250,
        }
        for text, expected in cases.items():
            with self.subTest(duration=text):
                self.assertEqual(row(duration=text)["duration_ms"], expected)

    def test_the_reported_count_is_read_from_the_entry(self) -> None:
        self.assertEqual(row()["records_reported"], 12_400)


class AJobMustBelongToAManagedConnection(unittest.TestCase):
    def test_a_job_on_an_unmanaged_connection_is_refused(self) -> None:
        """The listing is instance-wide, so it carries jobs this install does
        not manage. Guessing a connector for one would file syncs on the wrong
        row."""
        planned = plan.sync_row(entry(connectionId="someone-else"), CONNECTORS, TICK)
        self.assertIsInstance(planned, plan.Skipped)
        self.assertIn("managed connection", planned.reason)

    def test_a_job_with_no_connection_is_refused(self) -> None:
        planned = plan.sync_row(entry(connectionId=None), CONNECTORS, TICK)
        self.assertIsInstance(planned, plan.Skipped)

    def test_the_connector_comes_from_the_map_not_the_name(self) -> None:
        planned = plan.sync_row(entry(), {CONNECTION: "renamed"}, TICK)
        self.assertEqual(planned["connector"], "renamed")


class CoverageSkipsWhatIsClosed(unittest.TestCase):
    def test_a_job_already_terminal_in_the_ledger_is_left_alone(self) -> None:
        planned = plan.plan_syncs(
            [entry()], CONNECTORS, TICK, frozenset({"8412"})
        )
        self.assertEqual(planned.rows, [])

    def test_a_job_not_yet_closed_is_recorded_again(self) -> None:
        planned = plan.plan_syncs([entry()], CONNECTORS, TICK, frozenset())
        self.assertEqual(len(planned.rows), 1)

    def test_refusals_are_reported_beside_the_rows(self) -> None:
        planned = plan.plan_syncs(
            [entry(), entry(jobId=7, lastUpdatedAt=None, startTime=None)],
            CONNECTORS,
            TICK,
            frozenset(),
        )
        self.assertEqual(len(planned.rows), 1)
        self.assertEqual(len(planned.skipped), 1)
        self.assertEqual(planned.skipped[0].job_id, "7")


class EveryRowClassFillsEveryColumn(unittest.TestCase):
    """The insert names no columns, so a row missing one would silently take a
    DEFAULT — and a row carrying an extra one would be rejected outright."""

    def test_the_three_classes_agree_on_their_columns(self) -> None:
        sync = row()
        snapshot = plan.plan_snapshot([CONNECTOR], TICK)[0]
        seal = plan.plan_seal(TICK)
        self.assertEqual(set(snapshot), set(sync))
        self.assertEqual(set(seal), set(sync))

    def test_a_snapshot_row_names_its_connector_and_nothing_else(self) -> None:
        snapshot = plan.plan_snapshot([CONNECTOR], TICK)[0]
        self.assertEqual(snapshot["connector"], CONNECTOR)
        self.assertEqual(snapshot["event"], plan.CONNECTOR_CONFIGURED)
        self.assertEqual(snapshot["job_id"], "")
        self.assertEqual(snapshot["status"], "")
        self.assertIsNone(snapshot["job_updated_at"])

    def test_the_seal_names_no_connector(self) -> None:
        seal = plan.plan_seal(TICK)
        self.assertEqual(seal["event"], plan.SWEEP_COMPLETED)
        self.assertEqual(seal["connector"], "")
        self.assertEqual(seal["tick_id"], TICK)

    def test_a_snapshot_holds_each_connector_once(self) -> None:
        rows = plan.plan_snapshot([CONNECTOR, CONNECTOR, "other"], TICK)
        self.assertEqual([r["connector"] for r in rows], ["example-tracker", "other"])


if __name__ == "__main__":
    unittest.main()


class TheListingsOwnShape(unittest.TestCase):
    """The fixture above is the mover's shape, not a convenient stand-in for it.

    A field the mover never sends refuses every real entry while every test
    passes, and the page then reports every connector as never synced — which
    reads exactly like a mover that has run no syncs at all.
    """

    def test_the_fixture_carries_exactly_the_keys_the_listing_serves(self) -> None:
        self.assertEqual(frozenset(entry()), SERVED_KEYS)

    def test_a_creation_stamp_does_not_place_a_job(self) -> None:
        """The listing filters on creation and reports none, so an entry
        carrying only a creation stamp is one the mover cannot have sent."""
        invented = {
            k: v for k, v in entry().items() if k not in ("lastUpdatedAt", "startTime")
        }
        invented["createdAt"] = "2026-08-27T08:00:00Z"

        planned = plan.sync_row(invented, CONNECTORS, TICK)

        self.assertIsInstance(planned, plan.Skipped)


class AJobStillRunningIsPlaceable(unittest.TestCase):
    """A running job can arrive with a start and no update stamp at all.

    Refusing it costs the one state the page most needs: the connector reads as
    last synced whenever it last finished, while a sync is in flight or stuck.
    The whole page exists to tell those apart.
    """

    #: A running job as the listing serves one: started, not finished, nothing
    #: reported yet, and no update stamp.
    RUNNING = {
        "jobId": 9001,
        "connectionId": CONNECTION,
        "jobType": "sync",
        "status": "running",
        "startTime": "2026-08-27T08:00:30Z",
        "duration": "PT46M18S",
        "bytesSynced": 0,
        "rowsSynced": 0,
    }

    def test_it_is_recorded_rather_than_skipped(self) -> None:
        planned = plan.sync_row(self.RUNNING, CONNECTORS, TICK)

        self.assertNotIsInstance(planned, plan.Skipped, planned)
        self.assertEqual(planned["status"], "running")

    def test_it_is_placed_at_its_start(self) -> None:
        """The listing places such an entry around its start — it serves the
        entry when filtered from that moment and withholds it when filtered
        from after — so the ledger must place it there too, or the watermark
        and the filter part company."""
        planned = plan.sync_row(self.RUNNING, CONNECTORS, TICK)

        self.assertEqual(planned["job_updated_at"], "2026-08-27 08:00:30.000")

    def test_an_update_stamp_still_wins_when_there_is_one(self) -> None:
        """The fallback must not outrank the field it stands in for."""
        self.assertEqual(row()["job_updated_at"], "2026-08-27 08:02:52.000")

    def test_it_outranks_the_finished_job_it_follows(self) -> None:
        """A sync that started after the last one finished is the newer fact,
        and the summary orders along the same column the planner writes."""
        finished = entry(jobId=8900, lastUpdatedAt="2026-08-27T07:00:00Z")
        planned = plan.plan_syncs(
            [finished, self.RUNNING], CONNECTORS, TICK, frozenset()
        )

        placed = {r["job_id"]: r["job_updated_at"] for r in planned.rows}
        self.assertEqual(len(placed), 2)
        self.assertGreater(placed["9001"], placed["8900"])


class AnIgnoredFilterIsVisible(unittest.TestCase):
    """The mover answers 200 and drops a parameter it does not recognise.

    So a filter renamed by a later release stops filtering rather than failing:
    the read restarts at the beginning of history and a capped pass never
    reaches the newest jobs. Counting what came back below the watermark is the
    only signal that separates that from a page with nothing new to say.
    """

    WATERMARK = "2026-08-27 08:00:00.000"

    def test_entries_below_the_watermark_are_counted(self) -> None:
        older = entry(jobId=1, lastUpdatedAt="2026-08-26T23:59:59Z")
        at_the_mark = entry(jobId=2, lastUpdatedAt="2026-08-27T08:00:00Z")
        newer = entry(jobId=3, lastUpdatedAt="2026-08-27T09:00:00Z")

        self.assertEqual(
            plan.unfiltered_count([older, at_the_mark, newer], self.WATERMARK), 1
        )

    def test_a_filtered_listing_counts_nothing(self) -> None:
        self.assertEqual(plan.unfiltered_count([entry()], self.WATERMARK), 0)

    def test_the_first_sweep_sent_no_watermark_so_nothing_is_out_of_place(self) -> None:
        older = entry(lastUpdatedAt="2020-01-01T00:00:00Z")

        self.assertEqual(plan.unfiltered_count([older], None), 0)

    def test_an_unplaceable_entry_is_not_evidence_of_an_ignored_filter(self) -> None:
        """It is refused for its own reason; counting it here would report a
        working filter as broken on every tick that meets one."""
        self.assertEqual(
            plan.unfiltered_count([entry(lastUpdatedAt=None)], self.WATERMARK), 0
        )
