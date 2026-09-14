"""Two instances that name one CronWorkflow stop the connector, not the tick.

The instance label drops a source id's repeat of its connector, so `main` and
`claude-team-main` collapse onto the same name. Applied one after the other,
both applies succeed and the second replaces the first's schedule under the same
object: the connector reads as scheduled while one of its instances has silently
stopped syncing.

A guard that is only defined is not a guard. What is pinned here is that it runs
across a connector's whole instance set BEFORE any instance of it is applied,
and that a connector it refuses is the only one that stops.

Run: pytest src/ingestion/reconcile-connectors/tests
"""

from __future__ import annotations

import subprocess
from pathlib import Path

from reconcile_inputs import plan_row

ROOT = Path(__file__).resolve().parents[1]

#: The real plan reader, naming, and guard; everything else answers harmlessly.
STUBS = """
log_init()          { :; }
log_close()         { :; }
log_run_summary()   { :; }
log_line()          { printf '%s\\n' "$*" >&2; }
log_event()         { :; }
reconcile__log()    { printf '%s\\n' "$*" >&2; }
sweep_run()         { :; }
reconcile_compute_tenant() { printf 'example-tenant'; }
reconcile_gc_orphans()              { printf 'GC\\n' >> "$CALLS"; }
reconcile_prune_removed_instances() { printf 'PRUNE\\n' >> "$CALLS"; }
_reconcile_one_connector()          { printf 'RECONCILE %s %s\\n' "$1" "$9" >> "$CALLS"; }
"""


def run_tick(rows: list[str], tmp_path: Path) -> tuple[int, list[str], str]:
    calls = tmp_path / "calls"
    # The plan reaches bash through `printf '%b'`, so its tabs travel as the
    # two characters `\t` and are interpreted there.
    plan = "\\n".join(row.replace("\t", "\\t") for row in rows)
    script = f"""
    set -uo pipefail
    export INSIGHT_NAMESPACE=insight
    export CONNECTORS_DIR="{ROOT}/../connectors"
    export AIRBYTE_URL=http://127.0.0.1:1
    export INSIGHT_TENANT_ID=example-tenant
    export CALLS="{calls}"
    source "{ROOT}/lib/reconcile.sh"
    {STUBS}
    disc_load_instances() {{ printf '%b\\n' '{plan}'; }}
    reconcile_run 0 1 0 "" ""
    """
    result = subprocess.run(
        ["bash", "-c", script], capture_output=True, text=True, check=False
    )
    recorded = calls.read_text(encoding="utf-8").splitlines() if calls.exists() else []
    return result.returncode, recorded, result.stderr


class TestACollapsedPairIsRefusedBeforeAnythingIsApplied:
    def test_neither_instance_reaches_the_loop(self, tmp_path: Path) -> None:
        code, calls, stderr = run_tick(
            [
                plan_row("claude-team", "main", "secret-a"),
                plan_row("claude-team", "claude-team-main", "secret-b"),
            ],
            tmp_path,
        )

        assert "RECONCILE claude-team main" not in calls
        assert "RECONCILE claude-team claude-team-main" not in calls
        assert code == 2, "the connector must be reported failed, not quietly skipped"
        assert "both name their CronWorkflow" in stderr

    def test_the_connectors_beside_it_still_reconcile(self, tmp_path: Path) -> None:
        """The refusal is per connector. One connector's ambiguous source ids
        are not a reason to stop reconciling the rest of the install."""
        code, calls, _ = run_tick(
            [
                plan_row("claude-team", "main", "secret-a"),
                plan_row("claude-team", "claude-team-main", "secret-b"),
                plan_row("gitlab", "gitlab-main", "secret-c"),
            ],
            tmp_path,
        )

        assert "RECONCILE gitlab gitlab-main" in calls
        assert code == 2


class TestDistinctInstancesAreLeftAlone:
    def test_both_reach_the_loop(self, tmp_path: Path) -> None:
        """The other half of the guard: two instances that render two names are
        the ordinary multi-instance case and must not be refused."""
        code, calls, stderr = run_tick(
            [
                plan_row("claude-team", "claude-team-main", "secret-a"),
                plan_row("claude-team", "claude-team-second", "secret-b"),
            ],
            tmp_path,
        )

        assert code == 0, stderr
        assert calls == [
            "RECONCILE claude-team claude-team-main",
            "RECONCILE claude-team claude-team-second",
            "PRUNE",
            "GC",
        ]

    def test_a_descriptor_with_no_secret_is_not_a_collision(
        self, tmp_path: Path
    ) -> None:
        """Its three instance columns are empty, and two empty source ids are
        not two instances naming one schedule."""
        code, calls, stderr = run_tick(
            [plan_row("alpha"), plan_row("beta")], tmp_path
        )

        assert code == 0, stderr
        assert calls == ["RECONCILE alpha ", "RECONCILE beta ", "PRUNE", "GC"]
