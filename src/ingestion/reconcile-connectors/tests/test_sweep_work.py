"""Which connector instances a tick calls configured.

The sweep's snapshot is what the page prints as "these are your connectors", and
the read surface treats a sealed snapshot as authoritative. So the set is worth
a test of its own: too wide and the page lists connectors the install does not
have, too narrow and a configured one reads as removed.

Drives the real bash rather than a transcription of it — what the plan's shape
does to the snapshot is the whole point, and a rewrite in Python would test the
rewrite.

Run: pytest src/ingestion/reconcile-connectors/tests
"""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

CONNECTIONS = json.dumps(
    [
        {"name": "alpha-main-default-conn", "connectionId": "conn-alpha"},
        {"name": "bravo-main-default-conn", "connectionId": "conn-bravo"},
        {"name": "alpha-second-default-conn", "connectionId": "conn-alpha-second"},
    ]
)


def plan_row(name: str, source_id: str = "main", secret: str = "insight-secret") -> str:
    """One `disc_load_instances` row.

    Ten columns: the descriptor's seven, then the instance's three. Written with
    escapes because the stub renders it through `printf '%b'` — a literal tab
    here would be indistinguishable from the padding around it.
    """
    namespace = "bronze_" + name.replace("-", "_")
    return "\\t".join(
        [name, "dir", "1", "nocode", "", "", "", namespace, source_id, secret, "hash"]
    )


#: One instance each of two connectors, and a third the install does not
#: configure — the plan carries it with its instance columns empty.
DEFAULT_PLAN = "\\n".join(
    [plan_row("alpha"), plan_row("bravo"), plan_row("charlie", source_id="", secret="")]
)


def build_work(plan: str = DEFAULT_PLAN, *, unreadable: bool = False) -> subprocess.CompletedProcess:
    """Call `sweep__build_work` with its collaborators replaced.

    The plan is the one thing each case varies. Everything else is stubbed to a
    fixed answer so a failure can only be the function under test.
    """
    loader = "return 1" if unreadable else f"printf '%b\\n' {json.dumps(plan)}"
    script = f"""
    set -uo pipefail
    source "{ROOT}/lib/sweep.sh"
    log_line() {{ printf '%s\\n' "$*" >&2; }}
    disc_load_instances() {{ {loader}; }}
    reconcile_compute_tenant() {{ printf 'default'; }}
    reconcile_compute_connection_name() {{ printf '%s-%s-default-conn' "$1" "$2"; }}
    sweep__build_work "tick-1" {json.dumps(CONNECTIONS)}
    """
    return subprocess.run(
        ["bash", "-c", script], capture_output=True, text=True, check=False
    )


def names(result: subprocess.CompletedProcess) -> list[str]:
    work = json.loads(result.stdout)
    return [c["name"] for c in work["connectors"]]


class TestConfiguredMeansTheInstallHasIt:
    def test_a_descriptor_without_a_secret_is_not_configured(self) -> None:
        """Descriptors are every connector the product ships. Reporting them all
        fills the page with connectors this install never had — and reconcile
        agrees: without a Secret it deletes the source rather than driving it."""
        result = build_work()

        assert result.returncode == 0, result.stderr
        assert names(result) == ["alpha", "bravo"]

    def test_a_connector_with_a_secret_is_configured_even_with_no_connection(self) -> None:
        """A configured connector that never ran is a state the page must be
        able to show, so it is reported without a connection id, not dropped."""
        result = build_work("\\n".join([plan_row("alpha"), plan_row("delta")]))

        assert result.returncode == 0, result.stderr
        work = json.loads(result.stdout)
        by_name = {c["name"]: c for c in work["connectors"]}
        assert by_name["alpha"]["connection_id"] == "conn-alpha"
        assert "connection_id" not in by_name["delta"]

    def test_an_unreadable_plan_records_nothing_at_all(self) -> None:
        """A failed read is not an empty install. Recording one would seal a
        snapshot saying every connector is no longer configured, which the read
        surface takes at face value — so no snapshot is built."""
        result = build_work(unreadable=True)

        assert result.returncode == 1
        assert result.stdout.strip() == ""

    def test_no_descriptor_carries_a_secret_and_the_set_is_empty(self) -> None:
        """An empty set is not an error here. The caller refuses to seal it —
        an empty snapshot and "everything was removed" are the same rows."""
        result = build_work(plan_row("alpha", source_id="", secret=""))

        assert result.returncode == 0, result.stderr
        assert names(result) == []


class TestEveryInstanceIsItsOwnRow:
    def test_every_reported_connector_carries_its_instance_identity(self) -> None:
        """The sweep refuses work whose identity is absent, so a connector
        reported without one costs the whole tick its record — including the
        connectors whose identity was there."""
        result = build_work()

        assert result.returncode == 0, result.stderr
        work = json.loads(result.stdout)
        for connector in work["connectors"]:
            assert connector["tenant_id"] == "default", connector
            assert connector["source_id"] == "main", connector

    def test_two_instances_of_one_connector_are_two_entries(self) -> None:
        """They share a name and hold separate connections. Reported once, the
        page would show one row for the pair and the instance that synced last
        would stand for both."""
        result = build_work("\\n".join([plan_row("alpha"), plan_row("alpha", source_id="second")]))

        assert result.returncode == 0, result.stderr
        work = json.loads(result.stdout)
        reported = {(c["name"], c["source_id"], c.get("connection_id")) for c in work["connectors"]}
        assert reported == {
            ("alpha", "main", "conn-alpha"),
            ("alpha", "second", "conn-alpha-second"),
        }
