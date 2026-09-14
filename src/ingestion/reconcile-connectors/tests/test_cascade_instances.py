"""What the cascade deletes, and what it refuses to attribute to itself.

The cascade runs for a connector with no Secret at all, so it has to establish
two things before it deletes anything: which sources are this connector's, and
which instance each one is. Both have a wrong answer that costs live data — a
neighbour connector's sources, and a schedule named after an instance that was
guessed rather than read.

Run: pytest src/ingestion/reconcile-connectors/tests
"""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

from reconcile_inputs import Definition, Source, listing

ROOT = Path(__file__).resolve().parents[1]

CONNECTOR = "claude-team"
TENANT = "example-tenant"

#: Everything the cascade reaches for, recorded rather than performed.
STUBS = """
log_line()          { printf '%s\\n' "$*" >&2; }
log_event()         { :; }
reconcile__log()    { printf '%s\\n' "$*" >&2; }
reconcile_compute_tenant() { printf 'example-tenant'; }
ab_workspace_id()     { printf 'workspace-1'; }
ab_list_connections() { printf '[]'; }
ab_delete_source()    { printf 'DELETE-SOURCE %s\\n' "$1" >> "$CALLS"; }
disc_load_descriptors() {
  printf '%s\\tdir\\t1\\tnocode\\t\\t\\t\\tbronze\\n' claude-team claude-team-invoices gitlab
}
ab_list_definitions() { printf '%s' "$DEFINITIONS"; }
argo_delete_cronworkflow() {
  printf 'DELETE-CRONWORKFLOW %s %s\\n' "$1" "$3" >> "$CALLS"
}
argo_delete_superseded_cronworkflows() {
  printf 'DELETE-SUPERSEDED %s\\n' "$1" >> "$CALLS"
}
"""


#: A listing that reads and carries nothing of this connector's — the state
#: every legacy-shaped case below is about, where the source has no definition
#: to look up and ownership falls to its name. A listing that does not read at
#: all stops the pass outright; that is `test_ownership_unreadable.py`.
NOTHING_OF_OURS = [Definition("gitlab", "def-gitlab")]


def cascade(
    sources: list[Source],
    tmp_path: Path,
    connector: str = CONNECTOR,
    definitions: list[Definition] = NOTHING_OF_OURS,
) -> tuple[int, list[str], str]:
    calls = tmp_path / "calls"
    script = f"""
    set -uo pipefail
    export INSIGHT_NAMESPACE=insight
    export CONNECTORS_DIR="{ROOT}/../connectors"
    export AIRBYTE_URL=http://127.0.0.1:1
    export INSIGHT_TENANT_ID={TENANT}
    export CALLS="{calls}"
    export DEFINITIONS={json.dumps(listing(definitions))}
    source "{ROOT}/lib/reconcile.sh"
    {STUBS}
    ab_list_sources() {{ printf '%s' {json.dumps(listing(sources))}; }}
    reconcile_cascade_delete "{connector}"
    """
    result = subprocess.run(
        ["bash", "-c", script], capture_output=True, text=True, check=False
    )
    recorded = calls.read_text(encoding="utf-8").splitlines() if calls.exists() else []
    return result.returncode, recorded, result.stderr


class TestASourceThatNamesNoInstance:
    def test_its_source_goes_and_no_instance_schedule_is_guessed(
        self, tmp_path: Path
    ) -> None:
        code, calls, stderr = cascade(
            [Source(f"{CONNECTOR}-legacy", "src-legacy")], tmp_path
        )

        assert code == 0, stderr
        assert "DELETE-SOURCE src-legacy" in calls
        assert not any(line.startswith("DELETE-CRONWORKFLOW") for line in calls), (
            "an instance was invented for a source that named none"
        )
        assert f"DELETE-SUPERSEDED {CONNECTOR}" in calls, (
            "the shapes that name no instance are still the cascade's to remove"
        )
        assert "names no instance" in stderr, "and the operator is told which one"


class TestASourceThatNamesOne:
    def test_both_the_source_and_its_schedule_go(self, tmp_path: Path) -> None:
        """The other half: reading the instance out of the name is what the
        cascade does, and it must keep doing it."""
        code, calls, stderr = cascade(
            [Source(f"{CONNECTOR}-{CONNECTOR}-main-{TENANT}", "src-main")], tmp_path
        )

        assert code == 0, stderr
        assert "DELETE-SOURCE src-main" in calls
        assert f"DELETE-CRONWORKFLOW {CONNECTOR} {CONNECTOR}-main" in calls


class TestASiblingConnectorSharingTheNamePrefix:
    def test_its_sources_are_not_this_connectors_to_delete(
        self, tmp_path: Path
    ) -> None:
        """`claude-team` prefixes `claude-team-invoices`, and a source of the
        longer one begins with the shorter one's name plus a separator. Removing
        the shorter one's Secret must not take the longer one's data with it —
        the two are separate installations of separate connectors."""
        code, calls, stderr = cascade(
            [
                Source(f"{CONNECTOR}-{CONNECTOR}-main-{TENANT}", "src-main"),
                Source(
                    f"{CONNECTOR}-invoices-{CONNECTOR}-invoices-main-{TENANT}", "src-inv"
                ),
            ],
            tmp_path,
        )

        assert code == 0, stderr
        assert "DELETE-SOURCE src-main" in calls
        assert "DELETE-SOURCE src-inv" not in calls, (
            "the cascade deleted a source belonging to another connector"
        )
        assert not any("invoices" in line for line in calls if "CRONWORKFLOW" in line)


#: `claude-team` with the source id `invoices-main` is named exactly as an
#: instance of `claude-team-invoices` would be — the INVARIANT in
#: `python/airbyte_sources.py`. Both live, each with the definition this loop
#: published for its connector.
BOTH_LIVE = [
    Source(f"{CONNECTOR}-invoices-main-{TENANT}", "src-team-owned", "def-team"),
    Source(
        f"{CONNECTOR}-invoices-{CONNECTOR}-invoices-main-{TENANT}",
        "src-invoices-owned",
        "def-invoices",
    ),
]
BOTH_PUBLISHED = [
    Definition(CONNECTOR, "def-team"),
    Definition(f"{CONNECTOR}-invoices", "def-invoices"),
]


class TestANameTwoConnectorsCanSpell:
    def test_the_neighbours_cascade_leaves_it_alone(self, tmp_path: Path) -> None:
        """The destructive case: removing `claude-team-invoices`'s Secret must
        not take a live source of `claude-team` with it, however the name
        reads."""
        code, calls, stderr = cascade(
            BOTH_LIVE,
            tmp_path,
            connector=f"{CONNECTOR}-invoices",
            definitions=BOTH_PUBLISHED,
        )

        assert code == 0, stderr
        assert "DELETE-SOURCE src-invoices-owned" in calls
        assert "DELETE-SOURCE src-team-owned" not in calls, (
            "the cascade deleted a live source of the connector whose name this "
            "one merely prefixes"
        )

    def test_its_own_cascade_takes_it(self, tmp_path: Path) -> None:
        """The other half: the source really is `claude-team`'s, so removing
        `claude-team`'s Secret must take it — and its instance is
        `invoices-main`, not `main`."""
        code, calls, stderr = cascade(BOTH_LIVE, tmp_path, definitions=BOTH_PUBLISHED)

        assert code == 0, stderr
        assert "DELETE-SOURCE src-team-owned" in calls
        assert "DELETE-SOURCE src-invoices-owned" not in calls
        assert f"DELETE-CRONWORKFLOW {CONNECTOR} invoices-main" in calls

    def test_a_source_outside_the_listing_is_claimed_by_neither(
        self, tmp_path: Path
    ) -> None:
        """Fail closed per source: the listing reads, but this source's
        definition is not in it — deleted, or never published as ours. The name
        is then all there is, two connectors can spell it, and so neither may
        delete it. (A listing that does not read at all stops the whole pass;
        that is `test_ownership_unreadable.py`.)"""
        code, calls, stderr = cascade(BOTH_LIVE, tmp_path, definitions=NOTHING_OF_OURS)

        assert code == 0, stderr
        assert "DELETE-SOURCE src-team-owned" not in calls
