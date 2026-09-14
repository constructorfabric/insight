"""Adoption is a single-instance pass, and it says so instead of guessing.

`adopt` annotates resources that predate the annotations, and it reads which
instance they belong to from the one Secret the connector has. With two Secrets
there is nothing in those resources to tell the instances apart: the connections
would be tagged with whichever cfg-hash was listed first and the schedule named
after that instance, and both would look correct afterwards.

Making adoption instance-aware is not the answer either — there are no
multi-instance resources predating instances existing. Refusing is.

Run: pytest src/ingestion/reconcile-connectors/tests
"""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

CONNECTOR = "claude-team"
DEFINITION = "def-1"
AIRBYTE_SOURCE = "src-1"
CONNECTION = "conn-1"

DEFINITIONS = json.dumps(
    [{"name": CONNECTOR, "sourceDefinitionId": DEFINITION, "custom": True}]
)
SOURCES = json.dumps([{"sourceId": AIRBYTE_SOURCE, "sourceDefinitionId": DEFINITION}])
CONNECTIONS = json.dumps([{"connectionId": CONNECTION, "sourceId": AIRBYTE_SOURCE, "tags": []}])

#: Every call that would change something, recorded rather than made.
STUBS = """
log_line()          { printf '%s\\n' "$*" >&2; }
log_event()         { :; }
adopt_warn_orphan() { printf '%s\\n' "$*" >&2; }
reconcile_compute_connection_name() { printf 'conn-name'; }
reconcile_compute_schedule()        { printf '0 4 * * *'; }
reconcile_compute_tenant()          { printf 'example-tenant'; }
adopt_match_definition() { printf 'DEFINITION %s\\n' "$1" >> "$CALLS"; }
adopt_tag_connection()   { printf 'TAG-CONNECTION %s %s\\n' "$1" "$2" >> "$CALLS"; }
argo_apply_cronworkflow() { printf 'APPLY-CRONWORKFLOW %s\\n' "$5" >> "$CALLS"; }
argo_cron_workflow_name() { printf 'cron-name'; }
"""


def adopt(secret_rows: list[str], tmp_path: Path) -> tuple[int, list[str], str]:
    """Run `_adopt_one_connector` for one connector against a stubbed cluster."""
    calls = tmp_path / "calls"
    secrets = "\\n".join(secret_rows)
    script = f"""
    set -uo pipefail
    export INSIGHT_NAMESPACE=insight
    export CONNECTORS_DIR="{ROOT}/../connectors"
    export AIRBYTE_URL=http://127.0.0.1:1
    export INSIGHT_TENANT_ID=example-tenant
    export CALLS="{calls}"
    source "{ROOT}/lib/reconcile.sh"
    {STUBS}
    disc_load_secrets() {{ printf '%b\\n' {json.dumps(secrets)}; }}
    _adopt_one_connector "{CONNECTOR}" dir 1 nocode "" "" "" \
      0 "" workspace-1 {json.dumps(DEFINITIONS)} {json.dumps(SOURCES)} {json.dumps(CONNECTIONS)}
    """
    result = subprocess.run(
        ["bash", "-c", script], capture_output=True, text=True, check=False
    )
    recorded = calls.read_text(encoding="utf-8").splitlines() if calls.exists() else []
    return result.returncode, recorded, result.stderr


def secret_row(source_id: str, name: str, cfg_hash: str = "hash") -> str:
    return "\\t".join([CONNECTOR, source_id, name, cfg_hash])


class TestTwoSecretsStopAdoption:
    def test_nothing_is_tagged_and_no_schedule_is_applied(self, tmp_path: Path) -> None:
        code, calls, _ = adopt(
            [
                secret_row("claude-team-main", "secret-main"),
                secret_row("claude-team-second", "secret-second"),
            ],
            tmp_path,
        )

        assert calls == [], f"adoption changed something it could not attribute: {calls}"
        assert code == 1, "and the connector must be reported failed, not skipped"

    def test_the_refusal_says_why(self, tmp_path: Path) -> None:
        _, _, stderr = adopt(
            [
                secret_row("claude-team-main", "secret-main"),
                secret_row("claude-team-second", "secret-second"),
            ],
            tmp_path,
        )

        assert "multi-instance adoption is unsupported" in stderr
        assert "Nothing was changed" in stderr


class TestOneSecretStillAdopts:
    def test_the_connection_is_tagged_and_the_schedule_applied(self, tmp_path: Path) -> None:
        """The other half of the guard: refusing must depend on the count, not
        be the only thing this path does now."""
        code, calls, stderr = adopt([secret_row("claude-team-main", "secret-main")], tmp_path)

        assert code == 0, stderr
        assert calls == [
            f"DEFINITION {DEFINITION}",
            f"TAG-CONNECTION {CONNECTION} hash",
            "APPLY-CRONWORKFLOW claude-team-main",
        ]

    def test_no_secret_is_a_skip_rather_than_a_failure(self, tmp_path: Path) -> None:
        """Unchanged: a descriptor this install never configured is not a fault."""
        code, calls, _ = adopt([], tmp_path)

        assert code == 0
        assert calls == []
