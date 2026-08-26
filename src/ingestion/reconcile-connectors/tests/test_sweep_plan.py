"""The sweep planner's rules (connector-health spec §3.2, Job Sweep).

Every case here is a rule the design turns on, not a mechanic: what the sweep
must still corroborate after it stops collecting counters, when timing may not
be taken as evidence, and why an empty configured set has to be representable.
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python"))

from sweep.sweep_plan import (
    CLAIMED,
    CONNECTOR_CONFIGURED,
    OUT_OF_BAND,
    SWEEP_COMPLETED,
    SYNC_COMPLETED,
    UNCLAIMED,
    duration_ms,
    ledger_status,
    plan_sweep,
)

PLANNER = Path(__file__).resolve().parents[1] / "python" / "sweep" / "sweep_plan.py"

CONNECTOR = "example-tool"
CONNECTION = "conn-1"


def request(**overrides):
    base = {
        "jobs": [],
        "connections": {CONNECTION: CONNECTOR},
        "ledger": [],
        "workflow_claims": {},
        "records_readable": True,
        "configured": [],
        "tick_run_id": "tick-1",
        "horizon_epoch": 1000,
    }
    base.update(overrides)
    return base


def job(job_id="77", status="succeeded", **overrides):
    base = {
        "jobId": job_id,
        "connectionId": CONNECTION,
        "status": status,
        "startTimeEpoch": 2000,
        "duration": "PT1M37S",
        "rowsSynced": 425,
    }
    base.update(overrides)
    return base


def ledger_row(job_id="77", claim=UNCLAIMED, has_counters=True, **overrides):
    base = {
        "job_id": job_id,
        "connector": CONNECTOR,
        "claim": claim,
        "has_counters": has_counters,
        "started_at_epoch": 2000,
        "status": "ok",
        "duration_ms": 97000,
        "records_moved": 425,
    }
    base.update(overrides)
    return base


def syncs(plan):
    return [row for row in plan.rows if row.event == SYNC_COMPLETED]


class TestAJobWithNoReadableStartTime:
    """The epoch is not a time the mover reported; it is a parse failure."""

    def test_a_job_with_an_unreadable_start_time_is_not_recorded(self):
        # Recording it puts a sync in 1970 on the page, and the claim decision
        # reads this same stamp — a bogus one would call the job unclaimed on
        # arithmetic rather than on evidence.
        plan = plan_sweep(request(jobs=[job(startTimeEpoch=0)]))

        assert syncs(plan) == []

    def test_it_is_counted_so_the_tick_can_say_what_it_skipped(self):
        plan = plan_sweep(request(jobs=[job(job_id="77", startTimeEpoch=0)]))

        assert plan.undatable_jobs == ["77"]


class TestAJobStillRunning:
    """A provisional row must not close the job that will get an outcome."""

    def test_a_running_job_is_recorded_again_once_it_finishes(self):
        # Tick 1 saw it running; tick 2 must record the outcome, or the page
        # shows this sync in flight for as long as the ledger keeps it.
        plan = plan_sweep(
            request(
                jobs=[job(status="succeeded")],
                ledger=[ledger_row(status="running", claim=CLAIMED)],
            )
        )

        (row,) = syncs(plan)
        assert row.status == "ok"

    def test_a_running_job_is_recorded_again_when_it_fails(self):
        plan = plan_sweep(
            request(jobs=[job(status="failed")], ledger=[ledger_row(status="running", claim=CLAIMED)])
        )

        (row,) = syncs(plan)
        assert row.status == "failed"

    @pytest.mark.parametrize("provisional", ["running", "queued", "a_word_added_later"])
    def test_a_job_in_any_non_terminal_state_is_recorded_again(self, provisional):
        # Fail-closed: only the outcomes that END a job count as coverage. A
        # status this build does not know must not close it, or the tick that
        # could record the real outcome skips it forever.
        plan = plan_sweep(
            request(
                jobs=[job(status="succeeded")],
                ledger=[ledger_row(status=provisional, claim=CLAIMED)],
            )
        )

        (row,) = syncs(plan)
        assert row.status == "ok", f"should re-cover a job left in {provisional!r}"

    def test_a_job_already_recorded_with_an_outcome_is_not_recorded_twice(self):
        plan = plan_sweep(request(jobs=[job(status="succeeded")], ledger=[ledger_row(status="ok", claim=CLAIMED)]))

        assert syncs(plan) == []


class TestCoverage:
    def test_a_job_the_ledger_does_not_hold_is_recorded_from_the_movers_history(self):
        plan = plan_sweep(request(jobs=[job()]))

        (row,) = syncs(plan)
        assert row.job_id == "77"
        assert row.connector == CONNECTOR
        assert row.status == "ok"
        assert row.records_moved == 425
        assert row.duration_ms == 97000

    def test_the_sweep_never_claims_to_have_measured_delivery(self):
        plan = plan_sweep(request(jobs=[job()]))

        (row,) = syncs(plan)
        assert not hasattr(row, "rows_landed"), (
            "only the pipeline can measure delivery; a swept row must not carry a count"
        )

    def test_a_job_already_holding_counters_is_not_collected_again(self):
        plan = plan_sweep(request(jobs=[job()], ledger=[ledger_row(claim=CLAIMED)]))

        assert syncs(plan) == []

    def test_a_job_whose_connection_is_gone_is_reported_not_recorded_anonymously(self):
        plan = plan_sweep(request(jobs=[job(connectionId="vanished")]))

        assert syncs(plan) == []
        assert plan.unmappable_jobs == ["77"]


class TestCorroboration:
    def test_not_collecting_counters_does_not_stop_corroboration(self):
        """The bug this rule exists for: skipping a covered job must not freeze
        its claim, or a job seen once while the records were unreachable would
        stay unclaimed forever."""
        plan = plan_sweep(request(jobs=[job()], ledger=[ledger_row(claim=UNCLAIMED)], workflow_claims={"77": "wf-9"}))

        (row,) = syncs(plan)
        assert row.claim == CLAIMED
        assert row.run_id == "wf-9", "the claim must carry the run it links to"

    def test_an_unclaimed_job_no_record_names_is_settled_as_out_of_band(self):
        plan = plan_sweep(request(ledger=[ledger_row(claim=UNCLAIMED)], records_readable=True))

        (row,) = syncs(plan)
        assert row.claim == OUT_OF_BAND

    def test_a_job_older_than_any_retained_record_is_not_called_manual(self):
        """A backfilled job can predate every surviving workflow record, so the
        records' silence about it is not evidence — it is just deletion."""
        plan = plan_sweep(request(jobs=[job(startTimeEpoch=500)], horizon_epoch=1000))

        (row,) = syncs(plan)
        assert row.claim == UNCLAIMED

    def test_a_job_inside_the_horizon_no_record_names_is_out_of_band(self):
        plan = plan_sweep(request(jobs=[job(startTimeEpoch=2000)], horizon_epoch=1000))

        (row,) = syncs(plan)
        assert row.claim == OUT_OF_BAND

    def test_a_claim_by_identity_beats_the_horizon(self):
        plan = plan_sweep(request(jobs=[job(startTimeEpoch=500)], horizon_epoch=1000, workflow_claims={"77": "wf-3"}))

        (row,) = syncs(plan)
        assert row.claim == CLAIMED, "an explicit claim needs no horizon"

    def test_unreadable_records_leave_the_question_open_rather_than_answered(self):
        plan = plan_sweep(request(jobs=[job()], records_readable=False))

        (row,) = syncs(plan)
        assert row.claim == UNCLAIMED, "a lost pipeline claim must never be read as a manual sync"

    def test_unreadable_records_emit_no_verdict_for_an_existing_unclaimed_row(self):
        plan = plan_sweep(request(ledger=[ledger_row(claim=UNCLAIMED)], records_readable=False))

        assert syncs(plan) == []

    def test_a_job_past_the_record_horizon_is_no_longer_retried(self):
        plan = plan_sweep(request(ledger=[ledger_row(claim=UNCLAIMED, started_at_epoch=500)], horizon_epoch=1000))

        assert syncs(plan) == [], "past the horizon unclaimed is final, not out-of-band"

    @pytest.mark.parametrize("settled", [CLAIMED, OUT_OF_BAND])
    def test_a_settled_job_is_not_re_corroborated(self, settled):
        plan = plan_sweep(request(ledger=[ledger_row(claim=settled)], workflow_claims={"77": "wf-9"}))

        assert syncs(plan) == []


class TestConfiguredSnapshot:
    def test_every_managed_connector_is_recorded_under_one_tick(self):
        plan = plan_sweep(request(configured=["alpha", "beta"], tick_run_id="tick-7"))

        configured = [row for row in plan.rows if row.event == CONNECTOR_CONFIGURED]
        assert [row.connector for row in configured] == ["alpha", "beta"]
        assert {row.run_id for row in configured} == {"tick-7"}

    def test_an_empty_configured_set_is_still_sealed(self):
        """Without the marker, removing the last connector would leave the
        previous snapshot authoritative and the connector reading configured."""
        plan = plan_sweep(request(configured=[]))

        assert plan.seal is not None
        assert plan.seal.event == SWEEP_COMPLETED
        assert [row for row in plan.rows if row.event == CONNECTOR_CONFIGURED] == []

    def test_the_seal_is_handed_back_separately_so_the_caller_writes_it_last(self):
        """The bug this shape prevents: the shell also writes storage
        observations under this tick's id, and a snapshot read keys on the newest
        SEALED tick. A marker inside `rows` lands before those observations
        exist, so every reader in that window sees blank storage everywhere."""
        plan = plan_sweep(request(configured=["alpha"]))

        assert SWEEP_COMPLETED not in [row.event for row in plan.rows]
        assert plan.seal is not None
        assert plan.seal.run_id == "tick-1"


class TestVocabularyAtTheBoundary:
    @pytest.mark.parametrize(
        ("mover", "ledger"),
        [
            ("succeeded", "ok"),
            ("failed", "failed"),
            ("cancelled", "cancelled"),
            ("running", "running"),
            ("incomplete", "failed"),
        ],
    )
    def test_the_movers_outcome_words_become_the_ledgers(self, mover, ledger):
        assert ledger_status(mover) == ledger, f"should map: {mover!r}"

    @pytest.mark.parametrize(
        ("duration", "expected"),
        [
            ("PT1M37S", 97000),
            ("PT17M31S", 1051000),
            ("PT0S", 0),
            ("PT2M", 120000),
            ("PT1H2M3S", 3723000),
            ("", 0),
            (None, 0),
            (12.5, 12500),
        ],
    )
    def test_the_movers_duration_becomes_milliseconds(self, duration, expected):
        assert duration_ms(duration) == expected, f"should convert: {duration!r}"


class TestCli:
    def test_the_planner_round_trips_json_over_stdio(self):
        payload = json.dumps(request(jobs=[job()], configured=["alpha"]))

        result = subprocess.run(
            [sys.executable, str(PLANNER)], input=payload, capture_output=True, text=True, check=True
        )

        out = json.loads(result.stdout)
        events = [row["event"] for row in out["rows"]]
        assert events == [SYNC_COMPLETED, CONNECTOR_CONFIGURED]
        assert out["seal"]["event"] == SWEEP_COMPLETED, "the seal travels beside the rows, not in them"
        assert out["unmappable_jobs"] == []
