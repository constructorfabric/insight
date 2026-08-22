"""Tests for the seed's `--full-refresh` of the incremental identity feeders.

The defect these protect against is silent, which is why it needs a test rather
than a reviewer's attention. A seed REPLACES the org, but two models it depends
on are incremental — `bamboohr__employees_snapshot` appends, and
`identity_inputs` admits only rows past the current max `_version`. Seed a stand
with a roster whose people are NEW to it and nothing errors anywhere: bronze
holds the new people, the feeders keep describing the previous accounts,
persons-seed resolves that stale set, and gold goes on serving the old org.

Two halves are asserted here:
  * the silver step asks for a full refresh (the fix), and
  * the deploy path does not (the Helm migrate Hook runs the same script with
    no arguments, and its incremental models must keep their boundary).

Run against the installed package (see the README's develop section):

    uv run --extra dev pytest tests
"""

from __future__ import annotations

import subprocess
from pathlib import Path
from types import SimpleNamespace
from typing import Any

import pytest

from insight_seed import silver

_SCRIPTS = Path(__file__).resolve().parents[3] / "scripts"
_MIGRATIONS = _SCRIPTS / "apply-ch-migrations.sh"

_STUB_SCRIPT_ENV = {"CLICKHOUSE_URL": "http://clickhouse.nonpresent:8123"}

ScriptRun = tuple[list[str], dict[str, str]]


class _StubClient:
    """Stands in for the ClickHouse client `silver.run` opens and closes."""

    server_version = "0.0.0-stub"

    def close(self) -> None:
        pass


@pytest.fixture
def script_runs(monkeypatch: pytest.MonkeyPatch) -> list[ScriptRun]:
    """argv and env of every shell script the module would have run."""
    runs: list[ScriptRun] = []

    def record(argv: list[str], **kwargs: Any) -> None:
        runs.append((list(argv), dict(kwargs["env"])))

    # Swap the module's handle, not stdlib's: the real subprocess.run stays
    # available to everything else running in this session.
    monkeypatch.setattr(silver, "subprocess", SimpleNamespace(run=record))
    monkeypatch.setattr(silver, "_script_env", lambda: dict(_STUB_SCRIPT_ENV))
    return runs


@pytest.fixture
def refresh_requests(monkeypatch: pytest.MonkeyPatch) -> list[dict[str, Any]]:
    """What each seed step asks `apply_ch_migrations` for, without running it."""
    requests: list[dict[str, Any]] = []

    def record(**kwargs: Any) -> None:
        requests.append(kwargs)

    monkeypatch.setattr(silver, "apply_ch_migrations", record)
    return requests


@pytest.fixture
def offline_silver_step(monkeypatch: pytest.MonkeyPatch) -> None:
    """Everything `silver.run` touches besides the migration script."""
    monkeypatch.setattr(silver, "apply_create_bronze_placeholders", lambda: None)
    monkeypatch.setattr(silver, "_ch_client", _StubClient)
    monkeypatch.setattr(silver, "generate_rows", lambda client: None)


def _only(items: list[Any], what: str) -> Any:
    assert len(items) == 1, f"expected exactly one {what}, got {len(items)}"
    return items[0]


def _run_migration_script(
    *args: str, env: dict[str, str] | None = None
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["bash", str(_MIGRATIONS), *args],
        capture_output=True,
        text=True,
        check=False,
        env=env,
    )


def test_default_passes_no_flag(script_runs: list[ScriptRun]) -> None:
    """The deploy path: bash <script>, nothing else."""
    silver.apply_ch_migrations()

    argv, _ = _only(script_runs, "script run")
    assert argv[0] == "bash"
    assert "--full-refresh" not in argv


def test_full_refresh_appends_the_flag(script_runs: list[ScriptRun]) -> None:
    silver.apply_ch_migrations(full_refresh=True)

    argv, _ = _only(script_runs, "script run")
    assert argv[-1] == "--full-refresh"


def test_flag_never_leaks_into_the_environment(script_runs: list[ScriptRun]) -> None:
    """DBT_FULL_REFRESH is reconcile-connectors' name; we must not set it.

    It is exported per sync there ("true" on a major bump), and env is
    inherited by every child — reusing it would couple the two subsystems.
    """
    silver.apply_ch_migrations(full_refresh=True)

    _, env = _only(script_runs, "script run")
    assert "DBT_FULL_REFRESH" not in env


def test_selector_still_travels_by_env(script_runs: list[ScriptRun]) -> None:
    silver.apply_ch_migrations(dbt_select="tag:gold +identity_inputs")

    _, env = _only(script_runs, "script run")
    assert env["DBT_GOLD_SELECT"] == "tag:gold +identity_inputs"


@pytest.mark.usefixtures("offline_silver_step")
def test_silver_step_full_refreshes_the_identity_feeders(
    refresh_requests: list[dict[str, Any]],
) -> None:
    silver.run()

    request = _only(refresh_requests, "migration request")
    assert request.get("full_refresh"), (
        "the silver step must full-refresh: it just replaced every silver "
        "relation the seed owns, so the incremental identity feeders have "
        "to be rebuilt from it rather than appended to"
    )
    assert silver.IDENTITY_INPUTS_SELECT in request["dbt_select"]


def test_gold_step_does_not_full_refresh(refresh_requests: list[dict[str, Any]]) -> None:
    """`gold` rebuilds serving tables over an unchanged silver."""
    silver.run_gold()

    request = _only(refresh_requests, "migration request")
    assert not request.get("full_refresh", False)


def test_unknown_argument_is_refused() -> None:
    done = _run_migration_script("--nope")

    assert done.returncode == 2
    assert "unknown argument" in done.stderr


def test_help_prints_the_entire_header_block() -> None:
    """A fixed-line slice truncates silently as soon as the header grows."""
    header = []
    for line in _MIGRATIONS.read_text().splitlines()[1:]:
        if not line.startswith("#"):
            break
        header.append(line.removeprefix("#").removeprefix(" "))

    done = _run_migration_script("--help")

    assert done.returncode == 0
    assert "--full-refresh" in done.stdout
    assert done.stdout.splitlines() == header, (
        "help output is not the whole header — the slice must end at the "
        "first non-comment line, not a hardcoded line number"
    )


def test_accepted_flag_falls_through_to_the_env_asserts() -> None:
    """Parsing --full-refresh must not short-circuit the script's contract.

    The stripped env is the point: the script must still reach its own
    CLICKHOUSE_* asserts rather than exit happy on a parsed flag.
    """
    done = _run_migration_script("--full-refresh", env={"PATH": "/usr/bin:/bin"})

    assert done.returncode != 0
    assert "CLICKHOUSE_URL" in done.stderr


def test_the_script_forwards_its_flags_to_the_dbt_run() -> None:
    """Parsing --full-refresh is useless if the run never receives it."""
    body = _MIGRATIONS.read_text()

    assert "_dbt_flags+=(--full-refresh)" in body
    run_lines = [ln for ln in body.splitlines() if "_dbt_flags[@]" in ln]
    assert run_lines, "the dbt invocation does not reference _dbt_flags"
