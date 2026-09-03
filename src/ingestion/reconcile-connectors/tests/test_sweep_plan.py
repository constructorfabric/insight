"""What a tick decides to write.

Run: pytest src/ingestion/reconcile-connectors/tests
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

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


class TestStatusKeepsTheMoversWord:
    @pytest.mark.parametrize("word", sorted(vocab.MOVER_STATUSES))
    def test_documented_statuses_are_stored_verbatim(self, word: str) -> None:
        assert row(status=word)["status"] == word

    @pytest.mark.parametrize("word", ["SUCCEEDED_PARTIALLY", "", None, 7, "  "])
    def test_an_undocumented_word_is_stored_as_unknown(self, word: object) -> None:
        assert row(status=word)["status"] == vocab.UNKNOWN, (
            f"should not pass through: {word!r}"
        )

    def test_case_and_padding_do_not_produce_an_unknown(self) -> None:
        assert row(status=" Succeeded ")["status"] == "succeeded"

    def test_unknown_does_not_close_a_job(self) -> None:
        """Coverage fails closed: a status we could not read is re-read."""
        assert not vocab.is_terminal(vocab.UNKNOWN)

    @pytest.mark.parametrize("word", ["pending", "running"])
    def test_an_unfinished_status_does_not_close_a_job(self, word: str) -> None:
        assert not vocab.is_terminal(word)


class TestAJobMustBePlaceable:
    @pytest.mark.parametrize("absent", [None, 0, -1, "yesterday", True])
    def test_a_job_with_neither_stamp_readable_is_refused(self, absent: object) -> None:
        """Both, because either one places the job. A case that blanks only the
        update stamp stops testing the refusal the moment a fallback exists."""
        planned = plan.sync_row(
            entry(lastUpdatedAt=absent, startTime=absent), CONNECTORS, TICK
        )

        assert isinstance(planned, plan.Skipped), (
            f"should refuse both stamps = {absent!r}"
        )

    def test_a_job_with_no_identity_is_refused(self) -> None:
        planned = plan.sync_row(entry(jobId=None), CONNECTORS, TICK)
        assert isinstance(planned, plan.Skipped)

    def test_an_entry_with_no_job_is_refused(self) -> None:
        planned = plan.sync_row({"connectionId": CONNECTION}, CONNECTORS, TICK)
        assert isinstance(planned, plan.Skipped)

    def test_a_refusal_carries_its_reason(self) -> None:
        planned = plan.sync_row(
            entry(lastUpdatedAt=None, startTime=None), CONNECTORS, TICK
        )
        assert "moment" in planned.reason


class TestAbsenceIsNotZero:
    def test_a_job_not_started_has_no_start_and_no_duration(self) -> None:
        bare = {
            "jobId": 1,
            "connectionId": CONNECTION,
            "status": "pending",
            "lastUpdatedAt": UPDATED,
        }
        planned = plan.sync_row(bare, CONNECTORS, TICK)
        assert planned["started_at"] is None
        assert planned["duration_ms"] is None
        assert planned["records_reported"] is None

    def test_a_reported_zero_survives_as_zero(self) -> None:
        """A sync that moved nothing is not a sync nobody counted."""
        assert row(rowsSynced=0)["records_reported"] == 0

    @pytest.mark.parametrize("absent", [None, "", "many", True])
    def test_no_reported_count_at_all_is_absent(self, absent: object) -> None:
        assert row(rowsSynced=absent)["records_reported"] is None

    def test_a_negative_count_is_absent_rather_than_recorded(self) -> None:
        assert row(rowsSynced=-1)["records_reported"] is None

    @pytest.mark.parametrize("absent", [float("nan"), float("inf"), float("-inf")])
    def test_a_non_finite_number_is_absent_rather_than_an_exception(
        self, absent: float
    ) -> None:
        """`int(inf)` raises, and one malformed field would cost the whole tick
        its seal — which stops the page's own clock."""
        assert row(duration=absent)["duration_ms"] is None
        assert row(rowsSynced=absent)["records_reported"] is None

    @pytest.mark.parametrize("empty", ["", "   "])
    def test_an_empty_job_identity_is_refused(self, empty: str) -> None:
        """Several such jobs would share one key and replace each other during
        resolution, so the page would answer with whichever landed last."""
        planned = plan.sync_row(entry(jobId=empty), CONNECTORS, TICK)

        assert isinstance(planned, plan.Skipped)

    @pytest.mark.parametrize("absent", ["1m37s", "P", "", None, "PT", "garbage"])
    def test_a_duration_that_will_not_parse_is_absent(self, absent: object) -> None:
        """Reading only some components would report a span shorter than the
        truth, which is worse than reporting none."""
        assert row(duration=absent)["duration_ms"] is None


class TestWhatIsRead:
    def test_the_stamps_are_read_as_iso_8601(self) -> None:
        planned = row()
        assert planned["job_updated_at"] == "2026-08-27 08:02:52.000"
        assert planned["started_at"] == "2026-08-27 08:00:30.000"

    def test_a_stamp_with_an_offset_is_normalised_to_utc(self) -> None:
        planned = row(lastUpdatedAt="2026-08-27T11:00:00+03:00")
        assert planned["job_updated_at"] == "2026-08-27 08:00:00.000"

    def test_the_placing_stamp_is_the_last_update_not_the_start(self) -> None:
        """Two stamps arrive and only one places the job.

        Reading the other would still produce a row, so nothing but naming the
        expected value catches a planner that took `startTime` for the axis the
        watermark moves along.
        """
        assert row()["job_updated_at"] == "2026-08-27 08:02:52.000"
        assert row()["job_updated_at"] != row()["started_at"]

    def test_an_epoch_number_is_not_a_stamp(self) -> None:
        """The listing sends strings. A number here would be a different
        endpoint's shape, and guessing at it would date every job to 1970."""
        planned = plan.sync_row(
            entry(lastUpdatedAt=1_700_000_000, startTime=1_700_000_000),
            CONNECTORS,
            TICK,
        )
        assert isinstance(planned, plan.Skipped)

    @pytest.mark.parametrize(
        ("text", "expected"),
        [
            ("PT1M37S", 97_000),
            ("PT2H3M4.5S", 7_384_500),
            ("P1DT1S", 86_401_000),
            ("PT0.25S", 250),
        ],
    )
    def test_the_duration_is_read_as_an_iso_8601_duration(
        self, text: str, expected: int
    ) -> None:
        assert row(duration=text)["duration_ms"] == expected

    def test_the_reported_count_is_read_from_the_entry(self) -> None:
        assert row()["records_reported"] == 12_400


class TestAJobMustBelongToAManagedConnection:
    def test_a_job_on_an_unmanaged_connection_is_refused(self) -> None:
        """The listing is instance-wide, so it carries jobs this install does
        not manage. Guessing a connector for one would file syncs on the wrong
        row."""
        planned = plan.sync_row(entry(connectionId="someone-else"), CONNECTORS, TICK)
        assert isinstance(planned, plan.Skipped)
        assert "managed connection" in planned.reason

    def test_a_job_with_no_connection_is_refused(self) -> None:
        planned = plan.sync_row(entry(connectionId=None), CONNECTORS, TICK)
        assert isinstance(planned, plan.Skipped)

    def test_the_connector_comes_from_the_map_not_the_name(self) -> None:
        planned = plan.sync_row(entry(), {CONNECTION: "renamed"}, TICK)
        assert planned["connector"] == "renamed"


class TestCoverageSkipsWhatIsClosed:
    def test_a_job_already_terminal_in_the_ledger_is_left_alone(self) -> None:
        planned = plan.plan_syncs(
            [entry()], CONNECTORS, TICK, frozenset({"8412"})
        )
        assert planned.rows == []

    def test_a_job_not_yet_closed_is_recorded_again(self) -> None:
        planned = plan.plan_syncs([entry()], CONNECTORS, TICK, frozenset())
        assert len(planned.rows) == 1

    def test_refusals_are_reported_beside_the_rows(self) -> None:
        planned = plan.plan_syncs(
            [entry(), entry(jobId=7, lastUpdatedAt=None, startTime=None)],
            CONNECTORS,
            TICK,
            frozenset(),
        )
        assert len(planned.rows) == 1
        assert len(planned.skipped) == 1
        assert planned.skipped[0].job_id == "7"


class TestEveryRowClassFillsEveryColumn:
    """The insert names no columns, so a row missing one would silently take a
    DEFAULT — and a row carrying an extra one would be rejected outright."""

    def test_the_three_classes_agree_on_their_columns(self) -> None:
        sync = row()
        snapshot = plan.plan_snapshot([CONNECTOR], TICK)[0]
        seal = plan.plan_seal(TICK)
        assert set(snapshot) == set(sync)
        assert set(seal) == set(sync)

    def test_a_snapshot_row_names_its_connector_and_nothing_else(self) -> None:
        snapshot = plan.plan_snapshot([CONNECTOR], TICK)[0]
        assert snapshot["connector"] == CONNECTOR
        assert snapshot["event"] == plan.CONNECTOR_CONFIGURED
        assert snapshot["job_id"] == ""
        assert snapshot["status"] == ""
        assert snapshot["job_updated_at"] is None

    def test_the_seal_names_no_connector(self) -> None:
        seal = plan.plan_seal(TICK)
        assert seal["event"] == plan.SWEEP_COMPLETED
        assert seal["connector"] == ""
        assert seal["tick_id"] == TICK

    def test_a_snapshot_holds_each_connector_once(self) -> None:
        rows = plan.plan_snapshot([CONNECTOR, CONNECTOR, "other"], TICK)
        assert [r["connector"] for r in rows] == ["example-tracker", "other"]


class TestTheListingsOwnShape:
    """The fixture above is the mover's shape, not a convenient stand-in for it.

    A field the mover never sends refuses every real entry while every test
    passes, and the page then reports every connector as never synced — which
    reads exactly like a mover that has run no syncs at all.
    """

    def test_the_fixture_carries_exactly_the_keys_the_listing_serves(self) -> None:
        assert frozenset(entry()) == SERVED_KEYS

    def test_a_creation_stamp_does_not_place_a_job(self) -> None:
        """The listing filters on creation and reports none, so an entry
        carrying only a creation stamp is one the mover cannot have sent."""
        invented = {
            k: v for k, v in entry().items() if k not in ("lastUpdatedAt", "startTime")
        }
        invented["createdAt"] = "2026-08-27T08:00:00Z"

        planned = plan.sync_row(invented, CONNECTORS, TICK)

        assert isinstance(planned, plan.Skipped)


class TestAJobStillRunningIsPlaceable:
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

        assert not isinstance(planned, plan.Skipped), planned
        assert planned["status"] == "running"

    def test_it_is_placed_at_its_start(self) -> None:
        """The listing places such an entry around its start — it serves the
        entry when filtered from that moment and withholds it when filtered
        from after — so the ledger must place it there too, or the watermark
        and the filter part company."""
        planned = plan.sync_row(self.RUNNING, CONNECTORS, TICK)

        assert planned["job_updated_at"] == "2026-08-27 08:00:30.000"

    def test_an_update_stamp_still_wins_when_there_is_one(self) -> None:
        """The fallback must not outrank the field it stands in for."""
        assert row()["job_updated_at"] == "2026-08-27 08:02:52.000"

    def test_it_outranks_the_finished_job_it_follows(self) -> None:
        """A sync that started after the last one finished is the newer fact,
        and the summary orders along the same column the planner writes."""
        finished = entry(jobId=8900, lastUpdatedAt="2026-08-27T07:00:00Z")
        planned = plan.plan_syncs(
            [finished, self.RUNNING], CONNECTORS, TICK, frozenset()
        )

        placed = {r["job_id"]: r["job_updated_at"] for r in planned.rows}
        assert len(placed) == 2
        assert placed["9001"] > placed["8900"]

    def test_it_records_no_duration(self) -> None:
        """The mover reports an unfinished job's duration as a running total —
        a zero where it has none — and this column holds finished measurements.
        Recorded as-is it says the sync took no time, which is the one reading
        an operator watching a stuck sync must not be given."""
        planned = plan.sync_row(self.RUNNING, CONNECTORS, TICK)

        assert planned["duration_ms"] is None

    def test_it_still_states_when_it_started(self) -> None:
        """The elapsed time the page reports is derived from this stamp, so
        withholding the duration must not withhold the start with it."""
        planned = plan.sync_row(self.RUNNING, CONNECTORS, TICK)

        assert planned["started_at"] == "2026-08-27 08:00:30.000"


class TestOnlyAnUnfinishedJobLosesItsDuration:
    @pytest.mark.parametrize("word", sorted(vocab.IN_FLIGHT_STATUSES))
    def test_every_in_flight_word_records_none(self, word: str) -> None:
        assert row(status=word, duration="PT1M37S")["duration_ms"] is None

    @pytest.mark.parametrize("word", sorted(vocab.TERMINAL_STATUSES))
    def test_every_finished_word_keeps_what_it_reported(self, word: str) -> None:
        assert row(status=word, duration="PT1M37S")["duration_ms"] == 97_000

    def test_a_word_that_could_not_be_read_keeps_its_duration(self) -> None:
        """An unreadable word may well name a finished job, and treating it as
        in flight would discard a measurement that was actually taken."""
        assert row(status="borked", duration="PT1M37S")["duration_ms"] == 97_000

    def test_a_finished_job_that_really_took_no_time_still_says_so(self) -> None:
        """Suppression is decided by the status, never by the value: a zero
        from a finished job is a measurement, not an absence."""
        assert row(status="succeeded", duration="PT0S")["duration_ms"] == 0


class TestAnIgnoredFilterIsVisible:
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

        assert plan.unfiltered_count([older, at_the_mark, newer], self.WATERMARK) == 1

    def test_a_filtered_listing_counts_nothing(self) -> None:
        assert plan.unfiltered_count([entry()], self.WATERMARK) == 0

    def test_the_first_sweep_sent_no_watermark_so_nothing_is_out_of_place(self) -> None:
        older = entry(lastUpdatedAt="2020-01-01T00:00:00Z")

        assert plan.unfiltered_count([older], None) == 0

    def test_an_unplaceable_entry_is_not_evidence_of_an_ignored_filter(self) -> None:
        """It is refused for its own reason; counting it here would report a
        working filter as broken on every tick that meets one."""
        assert plan.unfiltered_count([entry(lastUpdatedAt=None)], self.WATERMARK) == 0
