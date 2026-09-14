"""What a tick is asked to touch, and what it refuses to guess.

`--connector` keeps meaning what it always did — one connector, every instance
of it — so existing callers are unaffected by instances existing. `--source-id`
narrows inside that, and only inside it: a source id identifies an instance
within a connector and is not guaranteed unique across the install.

Run: pytest src/ingestion/reconcile-connectors/tests
"""

from __future__ import annotations

import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MAIN = ROOT / "main.sh"

#: What the libraries assert while being sourced. Pointed at nothing reachable
#: on purpose: arguments are settled before the endpoint is probed, so a usage
#: error must not depend on this address answering.
ENV = {
    "PATH": "/usr/bin:/bin",
    "AIRBYTE_URL": "http://127.0.0.1:1",
    "ARGO_SERVICE_ACCOUNT": "insight-reconcile",
    "INSIGHT_NAMESPACE": "insight",
    "CONNECTORS_DIR": str(ROOT.parent / "connectors"),
}


def run(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["bash", str(MAIN), *args], capture_output=True, text=True, check=False, env=ENV
    )


class TestTheScopeFlags:
    def test_a_bare_source_id_is_a_usage_error(self) -> None:
        """Not a global search. Two connectors may both have an instance called
        `main`, so looking for one without naming its connector either picks a
        connector by accident or touches several while claiming to have
        narrowed to one."""
        result = run("reconcile", "--source-id", "claude-team-main")

        assert result.returncode == 64
        assert "--source-id requires --connector" in result.stderr

    def test_a_source_id_with_no_value_is_a_usage_error(self) -> None:
        """A flag whose value is missing is a usage error like any other, and
        answers with usage and 64. Read through a `${2:?}` expansion instead, it
        would end the shell where it stands: no usage, and an exit code this CLI
        does not use for a bad invocation."""
        result = run("reconcile", "--connector", "claude-team", "--source-id")

        assert result.returncode == 64, result.stderr
        assert "--source-id requires ID" in result.stderr
        assert "Usage:" in result.stderr

    def test_a_connector_with_no_value_is_a_usage_error(self) -> None:
        result = run("reconcile", "--connector")

        assert result.returncode == 64, result.stderr
        assert "--connector requires NAME" in result.stderr

    def test_help_answers_without_reaching_the_mover(self) -> None:
        result = run("--help")

        assert result.returncode == 0, result.stderr
        assert "--source-id" in result.stdout
        assert "every instance of it" in result.stdout

    def test_an_unknown_flag_is_a_usage_error_before_anything_is_probed(self) -> None:
        result = run("--not-a-flag")

        assert result.returncode == 64
        assert "unknown arg" in result.stderr
        assert "unreachable" not in result.stderr, "the probe must not have run"

    def test_source_id_with_adopt_is_a_usage_error(self) -> None:
        """Adoption refuses a connector with more than one instance, so there is
        never an instance for this flag to choose between. Accepted and ignored,
        it would read as a narrowing that was applied."""
        result = run("adopt", "--connector", "claude-team", "--source-id", "claude-team-main")

        assert result.returncode == 64
        assert "--source-id applies to reconcile only" in result.stderr

    def test_a_scoped_run_gets_as_far_as_reaching_for_the_mover(self) -> None:
        """The pair is accepted, so the run fails where a run with no mover
        fails — not on its arguments."""
        result = run("reconcile", "--connector", "claude-team", "--source-id", "claude-team-main")

        assert result.returncode != 64, result.stderr
        assert "--source-id requires --connector" not in result.stderr
