"""Every column of the descriptor TSV reaches the variable it was emitted for.

`disc_load_descriptors` emits seven columns and its readers are plain `read`
loops, which fail two ways at once: TAB is IFS-whitespace, so a run of tabs
coalesces and every column after an empty one shifts left; and a reader with
fewer variables than columns puts the remainder — separators included — into the
last one it has. Both are silent, and both end at the same place: the dbt
selector the sync workflow is rendered with.

What is pinned here is the values, not the shape of the reader: a future column
added to the TSV fails this the moment a reader is not extended with it.

Run: pytest src/ingestion/reconcile-connectors/tests
"""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

NAMESPACE = "bronze_alpha"
SELECTOR = "tag:alpha"

#: Every collaborator `adopt_run` reaches for, answering harmlessly. The body
#: under test is the loop that reads the descriptors, so the per-connector work
#: is replaced by a record of the arguments it was handed.
STUBS = """
log_line()          { printf '%s\\n' "$*" >&2; }
log_event()         { :; }
ab_workspace_id()      { printf 'workspace-1'; }
ab_list_definitions()  { printf '[]'; }
ab_list_sources()      { printf '[]'; }
ab_list_connections()  { printf '[]'; }
_adopt_one_connector() {
  printf 'ADOPT name=%s type=%s cdk=%s dbt=%s\\n' "$1" "$4" "$5" "$6" >> "$CALLS"
}
"""


def adopt_over(rows: list[str], tmp_path: Path) -> list[str]:
    """Run the real `adopt_run` over a stubbed descriptor listing."""
    calls = tmp_path / "calls"
    listing = "\\n".join(rows)
    script = f"""
    set -uo pipefail
    export INSIGHT_NAMESPACE=insight
    export CONNECTORS_DIR="{ROOT}/../connectors"
    export AIRBYTE_URL=http://127.0.0.1:1
    export INSIGHT_TENANT_ID=example-tenant
    export CALLS="{calls}"
    source "{ROOT}/lib/reconcile.sh"
    {STUBS}
    disc_load_descriptors() {{ printf '%b\\n' {json.dumps(listing)}; }}
    adopt_run 0 ""
    """
    result = subprocess.run(["bash", "-c", script], capture_output=True, text=True, check=False)
    assert result.returncode == 0, result.stderr
    return calls.read_text(encoding="utf-8").splitlines() if calls.exists() else []


def descriptor(name: str, cdk: str = "") -> str:
    """One row in the seven columns `disc_load_descriptors` emits."""
    return "\\t".join([name, "dir", "1", "nocode", cdk, SELECTOR, NAMESPACE])


class TestTheAdoptionLoopReadsEveryColumn:
    def test_the_namespace_does_not_land_in_the_dbt_selector(self, tmp_path: Path) -> None:
        """The seventh column has no reader in this pass, which is not the same
        as having no variable: without one it joins the sixth, and the
        selector the CronWorkflow is rendered with becomes two columns and a
        separator."""
        calls = adopt_over([descriptor("alpha")], tmp_path)

        assert calls == [f"ADOPT name=alpha type=nocode cdk= dbt={SELECTOR}"]
        assert NAMESPACE not in calls[0], "the connector's ClickHouse namespace reached a field that is not it"

    def test_an_empty_column_does_not_shift_the_ones_after_it(self, tmp_path: Path) -> None:
        """The image column is empty for nocode connectors, and under TAB it
        vanishes rather than being read as empty — shifting later fields."""
        calls = adopt_over([descriptor("beta")], tmp_path)

        assert calls == [f"ADOPT name=beta type=nocode cdk= dbt={SELECTOR}"]
