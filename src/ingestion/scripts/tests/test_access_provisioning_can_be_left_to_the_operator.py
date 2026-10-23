"""Access provisioning is a decision apart from database creation.

A clustered warehouse that stores access entities locally cannot take roles
and users from one deploy: they would land on the replica that answered. The
operator creates them on every replica instead and turns the provisioning off,
and then two things must hold. Every provisioning and revoking script is
skipped, from both Jobs that run `create-databases.sh`, while the databases are
still created and every other guard still runs. And the access-storage guard
becomes a note, not a refusal: it names the value it was downgraded by.

The transport is stubbed and the provisioning scripts are shadowed by a shim
on PATH that records being called, so the test reads what the deploy would
have run rather than what the server would have answered.
"""

from __future__ import annotations

import shutil
import subprocess
from pathlib import Path

import pytest

SCRIPTS = Path(__file__).resolve().parents[1]
TOPOLOGY_LIB = SCRIPTS / "lib" / "ch-topology.sh"
CREATE_DATABASES = SCRIPTS / "create-databases.sh"

THE_VALUE = "clickhouse.provisionAccess"
REACHED_THE_END = "GUARD PASSED"

#: A healthy cluster whose access storage is whatever REPLICATED_DIRECTORIES
#: says. Every statement is appended to SQL_LOG.
SCRIPTED = """
_ch_http_query() {
  local sql
  sql="$(cat)"
  printf '%s\\n' "${sql}" >> "${SQL_LOG}"
  case "${sql}" in
    *clusterAllReplicas*system.tables*) printf '\\n' ;;
    *system.user_directories*) printf '%s\\n' "${REPLICATED_DIRECTORIES}" ;;
    *system.macros*) printf '2\\n' ;;
    *system.zookeeper*"/replicas"*) printf '0\\n' ;;
    *engine_full*) printf '\\n' ;;
    *system.databases*) printf '\\n' ;;
    *) printf '1\\n' ;;
  esac
}
"""


def run(
    body: str,
    *,
    provision_access: str | None,
    replicated_directories: str = "0",
    cluster_mode: str = "true",
    tmp_path: Path,
) -> tuple[subprocess.CompletedProcess[str], str, list[str]]:
    """Run `body` against the real library; return the result, the SQL sent
    and the provisioning scripts invoked (by basename)."""
    shims = tmp_path / "bin"
    shims.mkdir(exist_ok=True)
    calls = tmp_path / "calls.log"
    sql = tmp_path / "sql.log"
    shim = shims / "bash"
    shim.write_text(f'#!/bin/sh\nbasename "$1" >> "{calls}"\n')
    shim.chmod(0o755)

    exports = f"export CLICKHOUSE_PROVISION_ACCESS='{provision_access}'" if provision_access is not None else ""
    script = f"""
set -euo pipefail
export CLICKHOUSE_URL=http://127.0.0.1:1 CLICKHOUSE_USER=u CLICKHOUSE_PASSWORD=p
export CLICKHOUSE_CLUSTER_MODE={cluster_mode} CLICKHOUSE_CLUSTER_NAME=insight_cluster
{exports}
source "$TOPOLOGY_LIB"
{SCRIPTED}
{body}
echo '{REACHED_THE_END}'
"""
    real_bash = shutil.which("bash", path="/usr/bin:/bin")
    assert real_bash, "no bash on /usr/bin:/bin"

    result = subprocess.run(
        [real_bash, "-c", script],
        capture_output=True,
        text=True,
        env={
            "TOPOLOGY_LIB": str(TOPOLOGY_LIB),
            "PATH": f"{shims}:/usr/bin:/bin",
            "REPLICATED_DIRECTORIES": replicated_directories,
            "SQL_LOG": str(sql),
        },
        check=False,
    )
    sent = sql.read_text() if sql.exists() else ""
    invoked = calls.read_text().split() if calls.exists() else []

    return result, sent, invoked


DEPLOY = f'CLICKHOUSE_DATABASE=insight source "{CREATE_DATABASES}"'


@pytest.mark.parametrize("off", ["false", "0", "no", "off", "False"])
def test_with_access_left_to_the_operator_no_provisioning_script_runs(off: str, tmp_path: Path) -> None:
    result, _, invoked = run(DEPLOY, provision_access=off, replicated_directories="1", tmp_path=tmp_path)

    assert result.returncode == 0, result.stderr
    assert not invoked, f"a provisioning script ran with {THE_VALUE} off ({off!r}): {invoked}"
    assert THE_VALUE in result.stdout, f"the deploy must say why it provisioned nothing: {result.stdout!r}"


def test_the_databases_are_still_created_and_the_other_guards_still_run(tmp_path: Path) -> None:
    result, sent, _ = run(DEPLOY, provision_access="false", tmp_path=tmp_path)

    assert result.returncode == 0, result.stderr
    assert sent.count("CREATE DATABASE IF NOT EXISTS") == 9, sent
    for probe in ("system.clusters", "system.zookeeper", "system.macros", "system.databases"):
        assert probe in sent, f"{probe} was never read with {THE_VALUE} off"


def test_by_default_every_provisioning_script_runs(tmp_path: Path) -> None:
    """The one block, unchanged: the presentation and grafana readers, the
    MCP explorer and the assistant's reader -- or its revocation."""
    result, _, invoked = run(DEPLOY, provision_access=None, replicated_directories="1", tmp_path=tmp_path)

    assert result.returncode == 0, result.stderr
    assert invoked == ["provision-presentation-access.sh", "revoke-v3-access.sh", "provision-grafana-access.sh"], (
        invoked
    )


def test_a_missing_replicated_directory_is_a_note_when_access_is_the_operators(tmp_path: Path) -> None:
    result, sent, _ = run("ch_assert_topology insight", provision_access="false", tmp_path=tmp_path)

    assert result.returncode == 0, result.stderr
    assert REACHED_THE_END in result.stdout
    assert "system.user_directories" in sent, "the guard stopped reading the access backend"
    assert "WARN" in result.stderr and THE_VALUE in result.stderr, result.stderr


def test_a_missing_replicated_directory_is_refused_by_default_and_names_the_way_out(tmp_path: Path) -> None:
    result, _, _ = run("ch_assert_topology insight", provision_access=None, tmp_path=tmp_path)

    assert result.returncode != 0, result
    assert REACHED_THE_END not in result.stdout
    assert THE_VALUE in result.stderr, result.stderr
    assert "createDatabases" not in result.stderr, (
        "createDatabases only drops the pre-install Job; it is not the way out"
    )


def test_a_replicated_directory_needs_no_note(tmp_path: Path) -> None:
    result, _, _ = run(
        "ch_assert_topology insight", provision_access="false", replicated_directories="1", tmp_path=tmp_path
    )

    assert result.returncode == 0, result.stderr
    assert "WARN" not in result.stderr, result.stderr


def test_a_standalone_install_may_also_leave_access_to_the_operator(tmp_path: Path) -> None:
    result, sent, invoked = run(DEPLOY, provision_access="false", cluster_mode="false", tmp_path=tmp_path)

    assert result.returncode == 0, result.stderr
    assert not invoked, invoked
    assert sent.count("CREATE DATABASE IF NOT EXISTS") == 9, sent
