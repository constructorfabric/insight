"""The Jira bronze rebuild copies a table's engine; on a cluster that engine must replicate.

`CREATE TABLE copy ON CLUSTER AS bronze_jira.<t>` takes the source's engine
verbatim. A plain source — a table from before clusterMode was turned on —
yields a plain copy that replicates nothing, and the quorum counts that judge
the rebuild would read the one replica holding it. The topology guard refuses
such a table before any DDL; the rebuild checks again at the copy, and must not
read a failed lookup as a replicated one.

The function is lifted out of the deploy script and run against a stubbed
transport, since sourcing the script would run the deploy.
"""

from __future__ import annotations

import re
import subprocess
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parents[1]
EXEC_LIB = SCRIPTS / "lib" / "ch-exec.sh"
HEALS = SCRIPTS / "apply-ch-migrations.sh"

ASSERTION = "_jira_require_replicated_source"
REBUILD = "_jira_swap_in_rebuilt"


def function_body(name: str) -> str:
    match = re.search(rf"^{name}\(\) \{{\n.*?^\}}\n", HEALS.read_text(), re.MULTILINE | re.DOTALL)
    assert match, f"{HEALS.name} no longer defines {name}"
    return match.group(0)


def run(*, cluster_mode: str, engine: str) -> subprocess.CompletedProcess[str]:
    """`engine` is what the lookup answers; `<fail>` makes the lookup fail."""
    script = f"""
set -euo pipefail
export CLICKHOUSE_URL=http://127.0.0.1:1 CLICKHOUSE_USER=u CLICKHOUSE_PASSWORD=p
export CLICKHOUSE_CLUSTER_MODE={cluster_mode} CLICKHOUSE_CLUSTER_NAME=insight_cluster
source "{EXEC_LIB}"
_ch_http_query() {{ [[ "$ENGINE" == "<fail>" ]] && return 7; printf '%s\\n' "$ENGINE"; }}
{function_body(ASSERTION)}
{ASSERTION} jira_issue
echo 'COPY MADE'
"""
    return subprocess.run(
        ["bash", "-c", script],
        capture_output=True,
        text=True,
        env={"PATH": "/usr/bin:/bin", "ENGINE": engine},
        check=False,
    )


def test_a_replicated_source_is_copied() -> None:
    result = run(cluster_mode="true", engine="ReplicatedReplacingMergeTree")

    assert result.returncode == 0, result.stderr
    assert "COPY MADE" in result.stdout


def test_a_plain_source_on_a_cluster_stops_the_deploy_before_the_copy() -> None:
    result = run(cluster_mode="true", engine="ReplacingMergeTree")

    assert result.returncode != 0, result
    assert "COPY MADE" not in result.stdout
    assert "bronze_jira.jira_issue" in result.stderr and "ReplacingMergeTree" in result.stderr, result.stderr


def test_a_lookup_that_did_not_run_is_not_read_as_a_replicated_source() -> None:
    result = run(cluster_mode="true", engine="<fail>")

    assert result.returncode != 0, result
    assert "COPY MADE" not in result.stdout
    assert "bronze_jira.jira_issue" in result.stderr, result.stderr


def test_a_standalone_rebuild_copies_the_plain_engine_it_expects() -> None:
    result = run(cluster_mode="false", engine="ReplacingMergeTree")

    assert result.returncode == 0, result.stderr
    assert "COPY MADE" in result.stdout


def test_the_assertion_runs_before_the_copy_is_made() -> None:
    body = function_body(REBUILD)

    assert ASSERTION in body, f"{REBUILD} no longer asserts its source's engine"
    assert body.index(ASSERTION) < body.index("CREATE TABLE ${copy}"), "the copy is made before its engine is known"
    called = [line.strip() for line in body.splitlines() if line.strip().startswith(ASSERTION)]
    assert called and all("||" not in call and "if " not in call for call in called), (
        f"`f || ...` turns `set -e` off inside the assertion; it must stop the deploy plainly: {called}"
    )
