"""The sweep's input readers and its SQL renderer (connector-health §3.2).

These are the pure pieces the shell wraps: what a workflow record proves, how
the configured set is decided, and the one place ledger SQL is written.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python"))

from sweep.sweep_claims import UNREADABLE, read_claims
from sweep.sweep_connections import join
from sweep.sweep_insert import statement
from sweep.sweep_request import build, epoch, ledger_row

SWEEP_SH = Path(__file__).resolve().parents[1] / "lib" / "sweep.sh"


def workflow(name="wf-1", created="2026-08-25T09:00:00Z", trigger_result="77", nodes=None):
    if nodes is None:
        nodes = {"n1": {"displayName": "trigger", "outputs": {"result": trigger_result}}}
    return {"metadata": {"name": name, "creationTimestamp": created}, "status": {"nodes": nodes}}


class TestWorkflowClaims:
    def test_a_run_claims_the_job_its_sync_step_triggered(self):
        result = read_claims({"items": [workflow(name="wf-9", trigger_result="1083")]})

        assert result["claims"] == {"1083": "wf-9"}
        assert result["readable"] is True

    def test_a_run_that_never_triggered_a_job_claims_nothing(self):
        nodes = {"n1": {"displayName": "resolve-connection-by-name", "outputs": {"exitCode": "1"}}}

        assert read_claims({"items": [workflow(nodes=nodes)]})["claims"] == {}

    def test_no_retained_records_still_answers_the_question(self):
        """An empty listing is an answer — nothing claims anything — while an
        unreadable listing is not, and the two must not look alike."""
        result = read_claims({"items": []})

        assert result["claims"] == {}
        assert result["readable"] is True
        assert UNREADABLE["readable"] is False

    def test_the_reader_does_not_infer_how_far_back_records_reach(self):
        """Retention there may be uneven, so the oldest surviving record is not
        a floor under anything. The horizon is a duration the caller supplies,
        never something read out of the data."""
        result = read_claims(
            {
                "items": [
                    workflow(name="new", created="2026-08-25T09:00:00Z", trigger_result="2"),
                    workflow(name="ancient", created="2026-04-28T09:00:00Z", trigger_result="1"),
                ]
            }
        )

        assert "horizon_epoch" not in result
        assert result["claims"] == {"2": "new", "1": "ancient"}


class TestConnectionJoin:
    def test_a_managed_connector_maps_to_its_connection_id(self):
        connections = [{"name": "alpha-alpha-main-default-conn", "connectionId": "id-1"}]

        result = join(connections, "alpha\talpha-alpha-main-default-conn")

        assert result["connections"] == {"id-1": "alpha"}
        assert result["configured"] == ["alpha"]

    def test_a_connector_the_mover_has_not_caught_up_with_is_still_configured(self):
        """Configured is a fact about this instance's intent, not about whether
        the mover has provisioned anything yet."""
        result = join([], "alpha\talpha-alpha-main-default-conn")

        assert result["configured"] == ["alpha"]
        assert result["connections"] == {}

    def test_a_connection_no_managed_connector_names_is_not_mapped(self):
        connections = [{"name": "stranger-conn", "connectionId": "id-9"}]

        assert join(connections, "")["connections"] == {}

    def test_a_connector_name_with_hyphens_survives_the_join(self):
        connections = [{"name": "claude-team-invoices-x-default-conn", "connectionId": "id-2"}]

        result = join(connections, "claude-team-invoices\tclaude-team-invoices-x-default-conn")

        assert result["connections"] == {"id-2": "claude-team-invoices"}


class TestRequestAssembly:
    def test_a_sweep_row_marks_a_job_collected_and_a_pipeline_row_does_not(self):
        collected = ledger_row(
            {
                "job_id": "1",
                "connector": "alpha",
                "claim": "claimed",
                "status": "ok",
                "has_counters": "1",
                "started_at_epoch": "10",
                "duration_ms": "20",
                "records_moved": "30",
            }
        )
        pipeline_only = ledger_row(
            {
                "job_id": "2",
                "connector": "alpha",
                "claim": "claimed",
                "status": "ok",
                "has_counters": "0",
                "started_at_epoch": "10",
                "duration_ms": "0",
                "records_moved": "0",
            }
        )

        assert collected["has_counters"] is True
        assert pipeline_only["has_counters"] is False, (
            "the pipeline's row carries the claim and the measurement, not the counters"
        )

    def test_job_start_stamps_become_epochs_the_planner_can_compare(self):
        request = build(
            jobs=[{"jobId": "1", "startTime": "2026-08-25T04:00:40Z"}],
            mapping={"connections": {}, "configured": []},
            ledger=[],
            claims={"claims": {}, "readable": True},
            tick_run_id="tick-1",
            horizon_epoch=5,
        )

        assert request["jobs"][0]["startTimeEpoch"] == int(epoch("2026-08-25T04:00:40Z"))
        assert request["horizon_epoch"] == 5
        assert request["tick_run_id"] == "tick-1"

    @pytest.mark.parametrize("bad", ["", None, "not-a-time"])
    def test_an_unparseable_stamp_is_zero_rather_than_an_exception(self, bad):
        assert epoch(bad) == 0, f"should tolerate: {bad!r}"


class TestInsertRendering:
    def row(self, **overrides):
        base = {
            "run_id": "tick-1",
            "job_id": "77",
            "connector": "alpha",
            "event": "sync.completed",
            "status": "ok",
            "origin": "sweep",
            "claim": "out_of_band",
            "started_at_epoch": 1700000000,
            "duration_ms": 97000,
            "records_moved": 425,
        }
        base.update(overrides)
        return base

    def test_nothing_to_write_renders_no_statement(self):
        assert statement("t", []) == "", "an empty tick must not send SQL"

    def test_a_row_renders_its_columns_and_a_typed_timestamp(self):
        sql = statement("ledger", [self.row()])

        assert sql.startswith("INSERT INTO ledger (")
        assert "'sync.completed'" in sql
        assert "toDateTime64(1700000000, 3)" in sql

    def test_several_rows_share_one_statement(self):
        sql = statement("ledger", [self.row(), self.row(job_id="78")])

        assert sql.count("toDateTime64") == 2
        assert sql.count("INSERT INTO") == 1

    @pytest.mark.parametrize(("value", "expected"), [("it's", "\\'"), ("a\\b", "\\\\"), ("plain", "plain")])
    def test_a_value_carrying_quotes_or_backslashes_is_escaped(self, value, expected):
        sql = statement("ledger", [self.row(connector=value)])

        assert expected in sql, f"should escape: {value!r}"

    def test_the_rendered_statement_survives_a_json_round_trip(self):
        rows = json.loads(json.dumps([self.row()]))

        assert statement("ledger", rows) == statement("ledger", [self.row()])


class TestTheCoverageFrontier:
    """The watermark is how far the SWEEP has read, not what the ledger holds."""

    def test_the_watermark_counts_only_rows_the_sweep_recorded(self):
        # The pipeline writes its own sync rows in real time. Counting those
        # puts the frontier at "now" on any running install, and the backfill
        # behind it is never requested again — FR-6 silently stops holding.
        source = SWEEP_SH.read_text()
        watermark = source.split("_sweep_watermark() {", 1)[1].split("\n}", 1)[0]

        assert "origin = 'sweep'" in watermark, (
            "the watermark must be the sweep's own coverage edge, or a running "
            "pipeline pushes it past history nothing has read"
        )

    def test_the_mover_listing_is_read_oldest_first(self):
        # Ascending order is what makes a truncated listing resumable: what is
        # cut is newer than everything collected, so the next tick continues at
        # the edge instead of leaving a hole behind the watermark.
        airbyte = (SWEEP_SH.parent / "airbyte.sh").read_text()

        assert "orderBy=createdAt%7CASC" in airbyte
        assert "orderBy=createdAt%7CDESC" not in airbyte
