from __future__ import annotations

import json
import shlex
from pathlib import Path

import pytest
import yaml
from test_connector_schedule_runs import ROOT, bash
from test_cronworkflow_error_logging import STUBS


@pytest.mark.parametrize("failure", ["secret", "descriptor", "saved"])
@pytest.mark.parametrize("bump_kind", ["major", "minor"])
@pytest.mark.parametrize("trigger_mode", ["enabled", "disabled", "dry-run"])
def test_schedule_failure_preserves_the_sync_decision_and_counts_one_failed_instance(
    tmp_path: Path, failure: str, bump_kind: str, trigger_mode: str
) -> None:
    directory = tmp_path / "area" / "example-connector"
    directory.mkdir(parents=True)
    descriptor = "schedule: [invalid yaml" if failure == "descriptor" else "schedule: '0 4 * * *'\n"
    (directory / "descriptor.yaml").write_text(descriptor)
    manifest = tmp_path / "sync.yaml"
    calls = tmp_path / "calls"
    calls.touch()
    summary = tmp_path / "summary"
    snapshot = "invalid JSON" if failure == "saved" else '{"schedules": []}'
    plan = f"example-connector\t{directory}\t2.0.0\tnocode\t\ttag:example\tbronze_example\tmain\texample-secret\thash"
    dry_run = int(trigger_mode == "dry-run")
    no_sync_trigger = int(trigger_mode == "disabled")
    result = bash(
        f"""
        set -euo pipefail
        source '{ROOT}/lib/reconcile.sh'
        {STUBS}
        unset -f reconcile_resolve_deployment_schedule reconcile_resolve_schedule
        source '{ROOT}/lib/connector-naming.sh'
        export ARGO_SERVICE_ACCOUNT=example-reconcile
        log_init() {{ :; }}
        log_close() {{ :; }}
        log_run_summary() {{ printf '%s\t%s\n' "$1" "$2" > '{summary}'; }}
        connector_schedules_read() {{
          _CONNECTOR_SCHEDULES_STATUS=ok
          _CONNECTOR_SCHEDULES_RESULT={shlex.quote(snapshot)}
        }}
        disc_load_instances() {{ printf '%s\n' {shlex.quote(plan)}; }}
        reconcile_definitions() {{ printf 'republish\t{bump_kind}\texample-definition\n'; }}
        reconcile_refresh_catalog() {{ printf 'catalog\n' >> '{calls}'; }}
        argo_apply_cronworkflow() {{ printf 'apply\n' >> '{calls}'; }}
        argo_delete_former_tenant_cronworkflows() {{ printf 'delete\n' >> '{calls}'; }}
        reconcile_prune_removed_instances() {{ :; }}
        reconcile_gc_orphans() {{ :; }}
        sweep_run() {{ :; }}
        kubectl() {{
          case "$*" in
            *' -o json') printf '{{"stringData": {{}}}}' ;;
            *' -o jsonpath='*) {"return 1" if failure == "secret" else "return 0"} ;;
            'create -f -') cat > '{manifest}'; printf 'sync\n' >> '{calls}' ;;
            *) printf 'unexpected %s\n' "$*" >> '{calls}'; return 1 ;;
          esac
        }}
        reconcile_run {dry_run} {no_sync_trigger}
        """,
        tmp_path,
    )

    assert result.returncode == 2, result.stdout + result.stderr
    recorded = calls.read_text().splitlines()
    expected_calls = ["catalog", "sync"] if trigger_mode == "enabled" else ["catalog"]
    assert recorded == expected_calls, result.stdout + result.stderr
    assert summary.read_text().split()[1] == "1", result.stdout + result.stderr
    logs = [json.loads(line) for line in result.stdout.splitlines()]
    errors = [entry["msg"] for entry in logs if entry["level"] == "error"]
    assert errors == [
        "example-connector: failed to resolve schedule; CronWorkflow was left unchanged",
        "example-connector: reconcile failed (continuing with next)",
    ], result.stdout + result.stderr

    if trigger_mode != "enabled":
        assert not manifest.exists()
        if trigger_mode == "dry-run":
            assert "would trigger a one-shot sync" in result.stdout
        return

    document = yaml.safe_load(manifest.read_text())
    assert document["kind"] == "Workflow"
    parameters = {parameter["name"]: parameter["value"] for parameter in document["spec"]["arguments"]["parameters"]}
    assert parameters["insight_source_id"] == "main"
    assert parameters["dbt_select"] == "tag:example"
    assert parameters["dbt_full_refresh"] == ("true" if bump_kind == "major" else "false")
