from __future__ import annotations

import shlex
import shutil
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[3]
SCRIPT = ROOT / "dev-compose.sh"
FIXTURE = ROOT / "tests/stand/fixtures/connector-schedules.sql"


def function(source: str, name: str) -> str:
    start = source.index(f"{name}() {{")
    return source[start : source.index("\n}\n", start) + 3]


@pytest.mark.parametrize("verb", ["up", "seed"])
def test_stand_setup_applies_the_schedule_fixture_on_every_run(tmp_path: Path, verb: str) -> None:
    env_file = tmp_path / "stand.env"
    env_file.touch()
    source = SCRIPT.read_text()
    functions = "\n".join(function(source, name) for name in ["test_stand_seed_connector_schedules", "cmd_test_stand"])
    setup = f"""
set -euo pipefail
ROOT_DIR={shlex.quote(str(ROOT))}
TEST_STAND_ENV_FILE={shlex.quote(str(env_file))}
COMPOSE_INSTANCE=stand-scratch-schedule
TEST_STAND_BUILD_FRONTEND=false
TEST_STAND_SKIP_BUILD=true
test_stand_select_instance() {{ :; }}
test_stand_resolve_build_modes() {{ :; }}
test_stand_prepare_env() {{ :; }}
test_stand_provide_backend() {{ :; }}
test_stand_origin() {{ printf 'http://localhost:18080'; }}
cmd_up() {{ :; }}
update_env_var() {{ :; }}
cmd_seed() {{ :; }}
test_stand_wait_ready() {{ :; }}
test_stand_published_port() {{ printf '18123'; }}
env_file_value() {{ printf 'stand-scratch-credential'; }}
curl() {{
  while [[ $# -gt 0 ]]; do
    if [[ "$1" == --data-binary ]]; then printf '%s\\n' "$2"; fi
    shift
  done
}}
"""
    result = subprocess.run(
        ["bash", "-c", f"{setup}\n{functions}\ncmd_test_stand {verb}\ncmd_test_stand {verb}"],
        capture_output=True,
        text=True,
        timeout=10,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    assert result.stdout.splitlines().count(f"@{FIXTURE}") == 2, result.stdout


@pytest.mark.parametrize("existing_history", [False, True])
def test_schedule_fixture_is_repeatable_with_sync_history(tmp_path: Path, existing_history: bool) -> None:
    clickhouse = shutil.which("clickhouse")
    if clickhouse is None:
        pytest.skip("ClickHouse local is required to exercise the fixture SQL")
    migration = (ROOT / "src/ingestion/scripts/migrations/20260827000000_connector-sync-history.sql").read_text()
    history = (
        "INSERT INTO ingestion_history.sync_events (tick_id, event) VALUES ('stand-scratch-other', 'sync.completed');"
        if existing_history
        else ""
    )
    query_file = tmp_path / "fixture-check.sql"
    query_file.write_text(
        "CREATE DATABASE ingestion_history;\n"
        + migration
        + history
        + FIXTURE.read_text() * 2
        + "SELECT event, count() FROM ingestion_history.sync_events "
        "GROUP BY event ORDER BY event FORMAT TSV;"
    )
    result = subprocess.run(
        [clickhouse, "local", "--path", str(tmp_path / "warehouse"), "--multiquery", "--queries-file", str(query_file)],
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    expected = "connector.configured\t1\nsweep.completed\t1\n"
    if existing_history:
        expected += "sync.completed\t1\n"
    assert result.stdout == expected
