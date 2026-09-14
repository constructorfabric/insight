"""What the removal passes do when they cannot establish whose a source is.

Both read ownership from the Airbyte definition listing. A listing that could
not be read is not an empty one: empty demotes every source to the name
fallback, and the name is the weaker evidence these passes exist not to delete
on. So a failed listing is not a degraded pass — it is no pass at all, and the
tick says so.

An empty array counts as failed for the same reason: Airbyte reports its bundled
definitions on every healthy call, and an error body that is valid JSON without
a `sourceDefinitions` key renders as `[]` and exits 0.

Run: pytest src/ingestion/reconcile-connectors/tests
"""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

import pytest
from reconcile_inputs import Definition, Source, listing, plan_row

ROOT = Path(__file__).resolve().parents[1]

CONNECTOR = "claude-team"
TENANT = "example-tenant"

#: A source this connector owns by definition, whose instance the plan below no
#: longer carries — so both passes have every reason to delete it, and only the
#: listing decides whether they may.
DOOMED = Source(f"{CONNECTOR}-{CONNECTOR}-gone-{TENANT}", "src-gone", "def-team")
PUBLISHED = [Definition(CONNECTOR, "def-team")]

#: The ways the listing can answer. The first three are the same answer: this
#: tick learned nothing about who owns what.
FAILS = "ab_list_definitions() { return 1; }"
EMPTY = "ab_list_definitions() { printf '[]'; }"
SILENT = "ab_list_definitions() { printf ''; }"
READS = 'ab_list_definitions() { printf \'%s\' "$DEFINITIONS"; }'

UNREADABLE = pytest.mark.parametrize(
    "definitions", [FAILS, EMPTY, SILENT], ids=["failed", "empty", "no bytes"]
)

STUBS = """
log_line()          { printf '%s\\n' "$*" >&2; }
log_event()         { :; }
reconcile__log()    { printf '%s\\n' "$*" >&2; }
reconcile_compute_tenant() { printf 'example-tenant'; }
ab_workspace_id()     { printf 'workspace-1'; }
ab_list_connections() { printf '[]'; }
ab_delete_source()    { printf 'DELETE-SOURCE %s\\n' "$1" >> "$CALLS"; }
disc_load_descriptors() {
  printf '%s\\tdir\\t1\\tnocode\\t\\t\\t\\tbronze\\n' claude-team claude-team-invoices
}
argo_delete_cronworkflow() {
  printf 'DELETE-CRONWORKFLOW %s %s\\n' "$1" "$3" >> "$CALLS"
}
argo_delete_instance_cronworkflow() {
  printf 'DELETE-CRONWORKFLOW %s %s\\n' "$1" "$3" >> "$CALLS"
}
argo_delete_superseded_cronworkflows() {
  printf 'DELETE-SUPERSEDED %s\\n' "$1" >> "$CALLS"
}
"""


def _run(call: str, definitions: str, tmp_path: Path) -> tuple[int, list[str], str]:
    calls = tmp_path / "calls"
    script = f"""
    set -uo pipefail
    export INSIGHT_NAMESPACE=insight
    export CONNECTORS_DIR="{ROOT}/../connectors"
    export AIRBYTE_URL=http://127.0.0.1:1
    export INSIGHT_TENANT_ID={TENANT}
    export CALLS="{calls}"
    export DEFINITIONS={json.dumps(listing(PUBLISHED))}
    source "{ROOT}/lib/reconcile.sh"
    {STUBS}
    {definitions}
    ab_list_sources() {{ printf '%s' {json.dumps(listing([DOOMED]))}; }}
    {call}
    """
    result = subprocess.run(
        ["bash", "-c", script], capture_output=True, text=True, check=False
    )
    recorded = calls.read_text(encoding="utf-8").splitlines() if calls.exists() else []
    return result.returncode, recorded, result.stderr


def cascade(definitions: str, tmp_path: Path) -> tuple[int, list[str], str]:
    return _run(f'reconcile_cascade_delete "{CONNECTOR}"', definitions, tmp_path)


def prune(definitions: str, tmp_path: Path) -> tuple[int, list[str], str]:
    plan = plan_row(CONNECTOR, f"{CONNECTOR}-main", "secret-main")
    return _run(
        f'reconcile_prune_removed_instances "{plan}" "" ""', definitions, tmp_path
    )


class TestTheCascade:
    @UNREADABLE
    def test_removes_nothing_at_all(self, definitions: str, tmp_path: Path) -> None:
        code, calls, stderr = cascade(definitions, tmp_path)

        assert calls == [], f"the cascade deleted without establishing ownership: {calls}"
        assert code == 1, "and the connector must be reported failed, not skipped"
        assert "cannot read the Airbyte definition listing" in stderr
        assert "removed nothing" in stderr

    def test_a_listing_that_reads_still_removes(self, tmp_path: Path) -> None:
        """The other half: the refusal must depend on the listing, not be the
        only thing this path does now."""
        code, calls, stderr = cascade(READS, tmp_path)

        assert code == 0, stderr
        assert "DELETE-SOURCE src-gone" in calls
        assert f"DELETE-CRONWORKFLOW {CONNECTOR} {CONNECTOR}-gone" in calls


class TestThePruningPass:
    @UNREADABLE
    def test_removes_nothing_at_all(self, definitions: str, tmp_path: Path) -> None:
        code, calls, stderr = prune(definitions, tmp_path)

        assert calls == [], f"the pass deleted without establishing ownership: {calls}"
        assert code == 0, "the pass is best-effort; it skips rather than failing the tick"
        assert "cannot read the Airbyte definition listing" in stderr
        assert "skipped" in stderr

    def test_a_listing_that_reads_still_prunes(self, tmp_path: Path) -> None:
        code, calls, stderr = prune(READS, tmp_path)

        assert code == 0, stderr
        assert "DELETE-SOURCE src-gone" in calls
        assert f"DELETE-CRONWORKFLOW {CONNECTOR} {CONNECTOR}-gone" in calls


class TestTheRestOfTheTick:
    def test_a_connector_whose_ownership_is_unreadable_does_not_stop_the_others(
        self, tmp_path: Path
    ) -> None:
        """The refusal is one connector's. The instances that are configured are
        reconciled as usual — the failure must not spread, and must not turn
        into a removal anywhere else either."""
        calls = tmp_path / "calls"
        plan = "\\n".join(
            row.replace("\t", "\\t")
            for row in [
                plan_row("claude-team"),
                plan_row("gitlab", "gitlab-main", "secret-b"),
            ]
        )
        script = f"""
        set -uo pipefail
        export INSIGHT_NAMESPACE=insight
        export CONNECTORS_DIR="{ROOT}/../connectors"
        export AIRBYTE_URL=http://127.0.0.1:1
        export INSIGHT_TENANT_ID={TENANT}
        export CALLS="{calls}"
        source "{ROOT}/lib/reconcile.sh"
        log_init()          {{ :; }}
        log_close()         {{ :; }}
        log_run_summary()   {{ :; }}
        log_line()          {{ printf '%s\\n' "$*" >&2; }}
        log_event()         {{ :; }}
        reconcile__log()    {{ printf '%s\\n' "$*" >&2; }}
        sweep_run()         {{ :; }}
        reconcile_compute_tenant() {{ printf 'example-tenant'; }}
        reconcile_gc_orphans() {{ printf 'GC\\n' >> "$CALLS"; }}
        reconcile_prune_removed_instances() {{ printf 'PRUNE\\n' >> "$CALLS"; }}
        ab_delete_source() {{ printf 'DELETE-SOURCE %s\\n' "$1" >> "$CALLS"; }}
        _reconcile_one_connector() {{
          if [[ -z "${{10}}" ]]; then
            reconcile_cascade_delete "$1"; return $?
          fi
          printf 'RECONCILE %s %s\\n' "$1" "$9" >> "$CALLS"
        }}
        ab_workspace_id()     {{ printf 'workspace-1'; }}
        ab_list_connections() {{ printf '[]'; }}
        ab_list_definitions() {{ return 1; }}
        ab_list_sources() {{ printf '%s' {json.dumps(listing([DOOMED]))}; }}
        disc_load_instances() {{ printf '%b\\n' '{plan}'; }}
        reconcile_run 0 1 0 "" ""
        """
        result = subprocess.run(
            ["bash", "-c", script], capture_output=True, text=True, check=False
        )
        recorded = calls.read_text(encoding="utf-8").splitlines() if calls.exists() else []

        assert "RECONCILE gitlab gitlab-main" in recorded, result.stderr
        assert not any(line.startswith("DELETE-") for line in recorded), (
            f"a tick that could not establish ownership still deleted: {recorded}"
        )
        assert result.returncode == 2, "and the tick reports the connector it could not do"
