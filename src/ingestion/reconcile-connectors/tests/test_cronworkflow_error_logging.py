from __future__ import annotations

import json
import os
import shlex
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
REASON = "example renderer refused the schedule"

STUBS = """
connector_schedules_acknowledge() { return 0; }
disc_instances_of() { printf 'main\texample-secret\thash\n'; }
valsec_check_secret() { return 0; }
reconcile_definitions() { printf 'noop\tnone\texample-definition\n'; }
reconcile_sources() { printf 'noop\texample-source\n'; }
reconcile_connections() { printf 'noop\texample-connection\n'; }
reconcile_rename_former_tenant() { return 0; }
reconcile_compute_connection_name() { printf 'example-connection'; }
reconcile_compute_tenant() { printf 'example-tenant'; }
reconcile_resolve_schedule() { printf 'default\n0 0 * * *'; }
adopt_match_definition() { return 0; }
adopt_tag_connection() { return 0; }
argo_render_cronworkflow() {
  printf '  example renderer refused the schedule\n\twith example detail\r\n  ' >&2
  return 1
}
kubectl() {
  if [[ "$*" == 'apply -f -' ]]; then
    printf 'unexpected apply\n' >&2
    return 1
  fi
  printf '{"stringData": {}}'
}
"""


def run_caller(caller: str, tmp_path: Path, overrides: str = "") -> subprocess.CompletedProcess[str]:
    (tmp_path / "descriptor.yaml").write_text("name: example-connector\n")
    definitions = json.dumps(
        [{"name": "example-connector", "sourceDefinitionId": "example-definition", "custom": True}]
    )
    sources = json.dumps([{"sourceId": "example-source", "sourceDefinitionId": "example-definition"}])
    connections = json.dumps([{"connectionId": "example-connection", "sourceId": "example-source", "tags": []}])
    calls = {
        "reconcile": f"""
          _reconcile_one_connector example-connector {shlex.quote(str(tmp_path))} 1 nocode '' '' \\
            bronze_example main example-secret hash 0 1 '' ''
        """,
        "adopt": f"""
          _adopt_one_connector example-connector {shlex.quote(str(tmp_path))} 1 nocode '' '' \\
            0 '' example-workspace {shlex.quote(definitions)} {shlex.quote(sources)} {shlex.quote(connections)}
        """,
    }
    script = f"""
    set -uo pipefail
    export INSIGHT_NAMESPACE=example-namespace CONNECTORS_DIR={shlex.quote(str(tmp_path))}
    export AIRBYTE_URL=http://127.0.0.1:1 INSIGHT_TENANT_ID=example-tenant
    source "{ROOT}/lib/reconcile.sh"
    {STUBS}
    {overrides}
    {calls[caller]}
    """
    return subprocess.run(
        ["bash", "-c", script],
        env={**os.environ, "PATH": f"{Path(sys.executable).parent}{os.pathsep}{os.environ['PATH']}"},
        capture_output=True,
        text=True,
        check=False,
    )


@pytest.mark.parametrize("caller", ["reconcile", "adopt"])
def test_renderer_refusal_reaches_the_error_log(caller: str, tmp_path: Path) -> None:
    result = run_caller(caller, tmp_path)

    assert result.returncode == 1, result.stdout + result.stderr
    logs = [json.loads(line) for line in result.stdout.splitlines()]
    errors = [entry["msg"] for entry in logs if entry["level"] == "error"]
    assert len(errors) == 1, result.stdout + result.stderr
    assert errors[0] == (f"example-connector: failed to create/update Argo CronWorkflow: {REASON} with example detail")
    assert result.stderr == ""


@pytest.mark.parametrize("caller", ["reconcile", "adopt"])
@pytest.mark.parametrize("apply_rc", [0, 1, 2])
def test_apply_status_keeps_its_meaning_and_stdout_is_discarded(caller: str, apply_rc: int, tmp_path: Path) -> None:
    overrides = f"""
    argo_apply_cronworkflow() {{
      printf 'example apply stdout that is not JSON\n'
      printf '  example apply reason\n  ' >&2
      return {apply_rc}
    }}
    argo_delete_former_tenant_cronworkflows() {{ return 0; }}
    """
    result = run_caller(caller, tmp_path, overrides)

    assert result.returncode == (0 if apply_rc == 0 else 1), result.stdout + result.stderr
    logs = [json.loads(line) for line in result.stdout.splitlines()]
    errors = [entry["msg"] for entry in logs if entry["level"] == "error"]
    assert len(errors) == (0 if apply_rc == 0 else 1), result.stdout + result.stderr
    assert result.stderr == ""
    if apply_rc == 0:
        return

    assert errors[0].endswith(": example apply reason")
    if apply_rc == 1:
        assert "failed to create/update Argo CronWorkflow" in errors[0]
    else:
        assert "failed to remove legacy CronWorkflow" in errors[0]
