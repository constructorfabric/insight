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
        "bytesSynced": 1024,
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
        "bytes_moved": 1024,
    }
    base.update(overrides)
    return base


def syncs(plan):
    return [row for row in plan.rows if row.event == SYNC_COMPLETED]


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
        """Found by running the sweep against a real instance: backfilled jobs
        predate every surviving workflow record, so the records' silence about
        them is not evidence — it is just deletion."""
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

        markers = [row for row in plan.rows if row.event == SWEEP_COMPLETED]
        assert len(markers) == 1
        assert [row for row in plan.rows if row.event == CONNECTOR_CONFIGURED] == []

    def test_the_marker_is_written_last_so_a_partial_snapshot_is_never_sealed(self):
        plan = plan_sweep(request(configured=["alpha"]))

        assert plan.rows[-1].event == SWEEP_COMPLETED


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
        assert events == [SYNC_COMPLETED, CONNECTOR_CONFIGURED, SWEEP_COMPLETED]
        assert out["unmappable_jobs"] == []
