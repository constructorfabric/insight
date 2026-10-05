"""Which destination-clickhouse versions reconcile agrees to build bronze with.

The definition is resolved by name at runtime, so an installation runs
whatever version its Airbyte carries. Below the floor the deploy has to stop:
the catalogs reconcile emits assume 2.x `append_dedup`, and a clustered
warehouse needs the replicated-engine fields that arrived in the named
release.

Run: pytest src/ingestion/reconcile-connectors/tests
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
CHECKER = ROOT / "python" / "check_destination_version.py"
PINS = ROOT.parents[0] / "scripts" / "bootstrap-db" / "pins.env"

MINIMUM = "2.1.29"

#: What reconcile_resolve_destination_id reaches for besides the gate.
STUBS = """
reconcile__log() {{ printf '%s\\n' "$*" >&2; }}
ab_destination_definition_by_name() {{ printf '%s\\t%s\\n' 'def-11111111' '{version}'; }}
ab_ensure_destination() {{ printf 'dest-22222222'; }}
"""

RECONCILE_ENV = """
export INSIGHT_NAMESPACE=insight
export CONNECTORS_DIR=/nonexistent
export AIRBYTE_URL=http://127.0.0.1:1
export RECONCILE_DEST_CLICKHOUSE_HOST=clickhouse.example.test
export RECONCILE_DEST_CLICKHOUSE_PORT=8123
export RECONCILE_DEST_CLICKHOUSE_DATABASE=insight
export RECONCILE_DEST_CLICKHOUSE_USERNAME=insight
export RECONCILE_DEST_CLICKHOUSE_PASSWORD=example-password
"""


def _check(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(CHECKER), *args], capture_output=True, text=True, encoding="utf-8", check=False
    )


def _resolve(version: str) -> subprocess.CompletedProcess[str]:
    script = f"""
    set -uo pipefail
    {RECONCILE_ENV}
    source "{ROOT}/lib/reconcile.sh"
    {STUBS.format(version=version)}
    reconcile_resolve_destination_id bronze
    """
    return subprocess.run(["bash", "-c", script], capture_output=True, text=True, encoding="utf-8", check=False)


@pytest.mark.parametrize("version", ["2.1.28", "2.1.0", "2.0.99", "1.9999.0", "2.1", "2"])
def test_a_version_below_the_minimum_is_refused(version: str) -> None:
    done = _check(version)

    assert done.returncode == 1, f"should refuse: {version!r}"
    assert MINIMUM in done.stdout


def test_the_minimum_itself_is_accepted() -> None:
    done = _check(MINIMUM)

    assert done.returncode == 0, done.stdout
    assert done.stdout == ""


@pytest.mark.parametrize("version", ["2.1.30", "2.1.29.1", "2.2.0", "3.0.0", "10.0.0"])
def test_a_version_above_the_minimum_is_accepted(version: str) -> None:
    done = _check(version)

    assert done.returncode == 0, f"should accept: {version!r} — {done.stdout}"
    assert done.stdout == ""


@pytest.mark.parametrize("version", ["2.1.29-dev.abc1234", "2.1.29-rc.1"])
def test_a_prerelease_does_not_satisfy_the_release_it_names(version: str) -> None:
    done = _check(version)

    assert done.returncode == 1, f"should refuse: {version!r}"


@pytest.mark.parametrize("version", ["", "latest", "dev", "v2.1.29", "sha256:0bad"])
def test_a_version_that_cannot_be_read_is_refused_rather_than_assumed_recent(version: str) -> None:
    done = _check(version)

    assert done.returncode == 1, f"should refuse: {version!r}"
    assert MINIMUM in done.stdout


def test_the_refusal_names_what_was_found_and_is_one_line_the_caller_can_log() -> None:
    done = _check("2.0.0")

    assert done.stdout.count("\n") == 1
    assert "2.0.0" in done.stdout
    assert MINIMUM in done.stdout


@pytest.mark.parametrize("args", [(), ("2.1.29", "2.1.30")])
def test_a_call_without_exactly_one_version_is_a_usage_error_not_a_refusal(args: tuple[str, ...]) -> None:
    """Exit 2 keeps a wiring mistake out of the operator-facing refusal."""
    done = _check(*args)

    assert done.returncode == 2
    assert done.stdout == ""


def test_the_snapshot_pin_is_never_older_than_the_floor() -> None:
    """Two statements of one requirement: the image the connector-table
    snapshot is generated with, and the oldest version an install may run."""
    pinned = re.search(r"^DESTINATION_CLICKHOUSE_IMAGE=.*:(\S+)$", PINS.read_text(encoding="utf-8"), re.MULTILINE)
    assert pinned is not None, f"no DESTINATION_CLICKHOUSE_IMAGE in {PINS}"

    done = _check(pinned.group(1))

    assert done.returncode == 0, done.stdout


class TestTheGateStopsReconcileAndNotOnlyTheHelper:
    def test_a_destination_below_the_minimum_never_reaches_the_create_call(self) -> None:
        done = _resolve("2.1.28")

        assert done.returncode == 1
        assert "dest-22222222" not in done.stdout
        assert "2.1.28" in done.stderr
        assert MINIMUM in done.stderr

    def test_a_destination_at_the_minimum_resolves(self) -> None:
        done = _resolve(MINIMUM)

        assert done.returncode == 0, done.stderr
        assert done.stdout == "dest-22222222"
