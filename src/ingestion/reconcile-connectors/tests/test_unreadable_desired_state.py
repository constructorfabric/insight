"""What a tick does when it cannot read what the install is supposed to have.

Every destructive path in the loop is driven by absence — a connector with no
row in the plan has its sources deleted, an instance with no row has its own
deleted. So a plan that is short for any reason other than "the operator removed
it" is the one input that can cost an install its data, and the only safe answer
is to reconcile nothing at all.

Drives the real `reconcile_run` with its collaborators replaced, so what is
asserted is the branch the loop actually takes.

Run: pytest src/ingestion/reconcile-connectors/tests
"""

from __future__ import annotations

import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

#: Everything `reconcile_run` reaches for, answering harmlessly and recording
#: that it was called. Only the plan read is varied per case.
STUBS = """
log_init()          { :; }
log_close()         { :; }
log_run_summary()   { :; }
log_line()          { printf '%s\\n' "$*" >&2; }
log_event()         { :; }
reconcile__log()    { printf '%s\\n' "$*" >&2; }
sweep_run()         { :; }
reconcile_gc_orphans()             { printf 'GC\\n' >> "$CALLS"; }
reconcile_prune_removed_instances() { printf 'PRUNE\\n' >> "$CALLS"; }
reconcile_cascade_delete()          { printf 'CASCADE %s\\n' "$1" >> "$CALLS"; }
_reconcile_one_connector()          { printf 'RECONCILE %s\\n' "$1" >> "$CALLS"; }
"""


def run_tick(loader: str, tmp_path: Path) -> tuple[int, list[str], str]:
    calls = tmp_path / "calls"
    script = f"""
    set -uo pipefail
    export INSIGHT_NAMESPACE=insight
    export CONNECTORS_DIR="{ROOT}/../connectors"
    export AIRBYTE_URL=http://127.0.0.1:1
    export CALLS="{calls}"
    source "{ROOT}/lib/reconcile.sh"
    {STUBS}
    disc_load_instances() {{ {loader}; }}
    reconcile_run 0 1 0 "" ""
    """
    result = subprocess.run(
        ["bash", "-c", script], capture_output=True, text=True, check=False
    )
    recorded = calls.read_text(encoding="utf-8").splitlines() if calls.exists() else []
    return result.returncode, recorded, result.stderr


class TestAnUnreadableDesiredStateReconcilesNothing:
    def test_no_connector_is_touched(self, tmp_path: Path) -> None:
        """Not one cascade, not one instance, not one removal pass. A read that
        failed answers "no Secret" for every connector at once, and that answer
        deletes every source the install has."""
        code, calls, _ = run_tick("return 1", tmp_path)

        assert calls == [], f"the tick acted on an unreadable plan: {calls}"
        assert code == 2, "and it must report itself failed, not quietly succeed"

    def test_the_reason_reaches_the_log(self, tmp_path: Path) -> None:
        code, _, stderr = run_tick("return 1", tmp_path)

        assert code == 2
        assert "cannot read the connector Secrets" in stderr

    def test_two_secrets_claiming_one_instance_stop_the_tick(self, tmp_path: Path) -> None:
        """The plan builder refuses that input rather than picking a winner, and
        a refused plan is an unreadable one as far as the loop is concerned."""
        code, calls, _ = run_tick("return 3", tmp_path)

        assert calls == []
        assert code == 2


class TestAReadableDesiredStateIsActedOn:
    def test_every_planned_instance_reaches_the_loop(self, tmp_path: Path) -> None:
        """The other half of the guard: a plan that read fine is not skipped."""
        plan = "\\n".join(
            [
                "alpha\\tdir\\t1\\tnocode\\t\\t\\t\\tbronze_alpha\\tmain\\tsecret-a\\thash",
                "alpha\\tdir\\t1\\tnocode\\t\\t\\t\\tbronze_alpha\\tsecond\\tsecret-b\\thash",
                "beta\\tdir\\t1\\tnocode\\t\\t\\t\\tbronze_beta\\t\\t\\t",
            ]
        )
        code, calls, stderr = run_tick(f"printf '%b\\\\n' '{plan}'", tmp_path)

        assert code == 0, stderr
        assert calls == [
            "RECONCILE alpha",
            "RECONCILE alpha",
            "RECONCILE beta",
            "PRUNE",
            "GC",
        ]
