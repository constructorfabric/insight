from __future__ import annotations

import json
import os
import shlex
import subprocess
import sys
from pathlib import Path

import pytest
from reconcile_inputs import Definition, Source, listing, plan_row
from test_connector_schedule_client import GOOD
from test_cronworkflow_error_logging import STUBS, run_caller
from test_render_cronworkflow import _render

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python"))

from sweep.schedule import read_schedule

ROOT = Path(__file__).resolve().parents[1]


def bash(script: str, tmp_path: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["bash", "-c", script],
        env={
            **os.environ,
            "INSIGHT_NAMESPACE": "example-namespace",
            "CONNECTORS_DIR": str(tmp_path),
            "AIRBYTE_URL": "http://127.0.0.1:1",
            "INSIGHT_TENANT_ID": "example-tenant",
            "PATH": f"{Path(sys.executable).parent}{os.pathsep}{os.environ['PATH']}",
        },
        capture_output=True,
        text=True,
        check=False,
    )


@pytest.mark.parametrize(
    ("status", "connector", "source_id", "annotation", "descriptor", "expected"),
    [
        ("ok", "example", "main", "0 1 * * *", "0 2 * * *", "api\n0 3 * * *\n0 15 * * *"),
        ("ok", "example", "second", "0 1 * * *", "0 2 * * *", "annotation\n0 1 * * *"),
        ("ok", "other", "main", "0 1 * * *", "0 2 * * *", "annotation\n0 1 * * *"),
        ("ok", "example", "second", "", "0 2 * * *", "descriptor\n0 2 * * *"),
        ("ok", "example", "second", "", "", "default\n0 0 * * *"),
    ],
)
def test_api_precedence_uses_both_identity_fields(
    tmp_path: Path, status: str, connector: str, source_id: str, annotation: str, descriptor: str, expected: str
) -> None:
    directory = tmp_path / "area" / connector
    directory.mkdir(parents=True)
    (directory / "descriptor.yaml").write_text(f"schedule: '{descriptor}'\n")
    result = bash(
        f"""
        set -euo pipefail
        source '{ROOT}/lib/connector-naming.sh'
        _CONNECTOR_SCHEDULES_STATUS={status}
        _CONNECTOR_SCHEDULES_RESULT={shlex.quote(json.dumps(GOOD))}
        kubectl() {{ printf '%s' {shlex.quote(annotation)}; }}
        reconcile_resolve_schedule {connector} example-secret {source_id}
        """,
        tmp_path,
    )
    assert result.returncode == 0, result.stderr
    assert result.stdout == expected


@pytest.mark.parametrize("caller", ["reconcile", "adopt"])
def test_unavailable_skips_each_instance_schedule_but_keeps_its_other_work(caller: str, tmp_path: Path) -> None:
    calls = tmp_path / "calls"
    overrides = f"""
    _CONNECTOR_SCHEDULES_STATUS=unavailable
    adopt_tag_connection() {{ printf 'tag\n' >> '{calls}'; }}
    reconcile_connections() {{ printf 'connection\n' >> '{calls}'; printf 'created\texample-connection\n'; }}
    argo_apply_cronworkflow() {{ printf 'apply\n' >> '{calls}'; }}
    argo_delete_former_tenant_cronworkflows() {{ printf 'delete\n' >> '{calls}'; }}
    """
    result = run_caller(caller, tmp_path, overrides)
    assert result.returncode == 0, result.stdout + result.stderr
    assert calls.read_text().splitlines() == (["tag"] if caller == "adopt" else ["connection"])


@pytest.mark.parametrize(
    ("caller", "scope"),
    [
        ("reconcile", "all"),
        ("reconcile", "connector"),
        ("reconcile", "instance"),
        ("adopt", "all"),
        ("adopt", "connector"),
    ],
)
@pytest.mark.parametrize("dry_run", [0, 1])
@pytest.mark.parametrize(
    ("client_result", "reason"),
    [
        ('{"status":"unavailable","reason":"example read refusal"}', "example read refusal"),
        ("not JSON", "invalid schedule client result"),
        ('{"status":"unexpected"}', "invalid schedule client result"),
    ],
    ids=["unavailable", "not-json", "unknown-status"],
)
def test_one_failed_read_leaves_schedules_unchanged_and_finishes_other_work(
    tmp_path: Path, caller: str, dry_run: int, scope: str, client_result: str, reason: str
) -> None:
    calls = tmp_path / "calls"
    client = tmp_path / "client"
    client.mkdir()
    (client / "read_connector_schedules.py").write_text(
        f"from pathlib import Path\np = Path({str(calls)!r})\n"
        "with p.open('a') as f: f.write('read\\n')\n"
        f"print({client_result!r})\n"
    )
    (tmp_path / "descriptor.yaml").write_text("name: example-connector\n")
    definitions = json.dumps(
        [
            {"name": name, "sourceDefinitionId": f"definition-{index}", "custom": True}
            for index, name in enumerate(["example-connector", "example-two"])
        ]
    )
    sources = json.dumps(
        [{"sourceId": f"source-{index}", "sourceDefinitionId": f"definition-{index}"} for index in range(2)]
    )
    connections = json.dumps(
        [{"connectionId": f"connection-{index}", "sourceId": f"source-{index}", "tags": []} for index in range(2)]
    )
    plan = "\n".join(
        f"{name}\t{tmp_path}\t1\tnocode\t\t\tbronze_example\t{source}\texample-secret\thash"
        for name, source in [("example-connector", "main"), ("example-connector", "second")]
    )
    descriptors = "\n".join(
        f"{name}\t{tmp_path}\t1\tnocode\t\t\tbronze_example" for name in ["example-connector", "example-two"]
    )
    connector_scope = "example-connector" if scope != "all" else ""
    source_scope = "main" if scope == "instance" else ""
    result = bash(
        f"""
        set -uo pipefail
        source '{ROOT}/lib/reconcile.sh'
        {STUBS}
        _CONNECTOR_SCHEDULES_PY_DIR='{client}'
        log_init() {{ :; }}
        log_close() {{ :; }}
        log_run_summary() {{ :; }}
        log_line() {{ printf '%s\n' "$*" >&2; }}
        disc_load_instances() {{ printf '%s' {shlex.quote(plan)}; }}
        disc_load_descriptors() {{ printf '%s' {shlex.quote(descriptors)}; }}
        ab_workspace_id() {{ printf 'example-workspace'; }}
        ab_list_definitions() {{ printf '%s' {shlex.quote(definitions)}; }}
        ab_list_sources() {{ printf '%s' {shlex.quote(sources)}; }}
        ab_list_connections() {{ printf '%s' {shlex.quote(connections)}; }}
        adopt_match_definition() {{ printf 'definition\n' >> '{calls}'; }}
        adopt_tag_connection() {{ printf 'tag\n' >> '{calls}'; }}
        reconcile_connections() {{ printf 'connection\n' >> '{calls}'; printf 'created\texample-connection\n'; }}
        argo_apply_cronworkflow() {{ printf 'apply\n' >> '{calls}'; }}
        argo_delete_former_tenant_cronworkflows() {{ printf 'delete\n' >> '{calls}'; }}
        argo_submit_sync_trigger() {{ printf 'sync\n' >> '{calls}'; }}
        reconcile_prune_removed_instances() {{ printf 'prune\n' >> '{calls}'; }}
        reconcile_gc_orphans() {{ printf 'gc\n' >> '{calls}'; }}
        sweep_run() {{ printf 'sweep\n' >> '{calls}'; }}
        if [[ '{caller}' == reconcile ]]; then
          reconcile_run {dry_run} 0 0 '{connector_scope}' '{source_scope}'
        else
          adopt_run {dry_run} '{connector_scope}'
        fi
        """,
        tmp_path,
    )
    assert result.returncode == 2, result.stdout + result.stderr
    recorded = calls.read_text().splitlines()
    assert recorded.count("read") == 1 and recorded[0] == "read", recorded
    assert "apply" not in recorded and "delete" not in recorded, recorded
    if caller == "reconcile":
        instance_count = 1 if scope == "instance" else 2
        assert recorded.count("connection") == instance_count, recorded
        assert recorded[-3:] == ["prune", "gc", "sweep"], recorded
        assert recorded.count("sync") == (0 if dry_run else instance_count), recorded
    else:
        connector_count = 2 if scope == "all" else 1
        assert recorded == (["read"] if dry_run else ["read"] + ["definition", "tag"] * connector_count), recorded
    errors = [line for line in result.stderr.splitlines() if line.startswith("ERROR")]
    assert errors == [f"ERROR connector schedules unavailable: {reason}; CronWorkflow schedules were left unchanged"], (
        result.stdout + result.stderr
    )


@pytest.mark.parametrize("caller", ["reconcile", "adopt"])
def test_dry_run_reports_the_selected_origin(caller: str, tmp_path: Path) -> None:
    result = bash(
        f"""
        set -uo pipefail
        source '{ROOT}/lib/reconcile.sh'
        {STUBS}
        _CONNECTOR_SCHEDULES_STATUS=ok
        _CONNECTOR_SCHEDULES_RESULT={shlex.quote(json.dumps({**GOOD, "schedules": [{**GOOD["schedules"][0], "connector": "example-connector"}]}))}
        unset -f reconcile_resolve_schedule
        source '{ROOT}/lib/connector-naming.sh'
        disc_instances_of() {{ printf 'main\texample-secret\thash\n'; }}
        disc_load_instances() {{ printf 'example-connector\t{tmp_path}\t1\tnocode\t\t\tbronze_example\tmain\texample-secret\thash'; }}
        disc_load_descriptors() {{ printf 'example-connector\t{tmp_path}\t1\tnocode\t\t\tbronze_example'; }}
        ab_workspace_id() {{ printf 'workspace'; }}
        ab_list_definitions() {{ printf '[{{"name":"example-connector","sourceDefinitionId":"definition","custom":true}}]'; }}
        ab_list_sources() {{ printf '[{{"sourceId":"source","sourceDefinitionId":"definition"}}]'; }}
        ab_list_connections() {{ printf '[{{"connectionId":"connection","sourceId":"source","tags":[]}}]'; }}
        connector_schedules_read() {{ :; }}
        log_init() {{ :; }}
        log_close() {{ :; }}
        log_run_summary() {{ :; }}
        sweep_run() {{ :; }}
        reconcile_prune_removed_instances() {{ :; }}
        reconcile_gc_orphans() {{ :; }}
        {caller}_run 1
        """,
        tmp_path,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    assert "origin=api" in result.stdout, result.stdout + result.stderr


@pytest.mark.parametrize(
    ("operation", "expected"),
    [
        ("argo_apply_cronworkflow example connection '0 3 * * *' tenant main '' api", ["apply", "example-tenant-sync"]),
        ("_argo_delete_cronworkflow_named example", ["example"]),
        ("argo_delete_cronworkflow example tenant main", ["example-tenant-sync", "example-main-tenant-sync"]),
        ("argo_delete_former_tenant_cronworkflows example tenant main", ["example-main-former-sync"]),
        ("argo_delete_instance_cronworkflow example tenant main", ["example-main-tenant-sync"]),
        ("argo_delete_superseded_cronworkflows example tenant", ["example-tenant-sync"]),
    ],
)
def test_argo_helpers_do_not_skip_explicit_mutations(tmp_path: Path, operation: str, expected: list[str]) -> None:
    calls = tmp_path / "calls"
    result = bash(
        f"""
        set -euo pipefail
        source '{ROOT}/lib/argo.sh'
        _CONNECTOR_SCHEDULES_STATUS=unavailable
        argo_render_cronworkflow() {{ printf 'example manifest'; }}
        kubectl() {{
          case "$*" in
            *' get '*) printf 'example-main-former-sync\tformer\n' ;;
            *' delete '*) printf '%s\n' "${{4#cronworkflow.argoproj.io/}}" >> '{calls}' ;;
            'apply -f -') cat >/dev/null; printf 'apply\n' >> '{calls}' ;;
            *) return 1 ;;
          esac
        }}
        {operation}
        """,
        tmp_path,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    recorded = calls.read_text().splitlines() if calls.exists() else []
    assert recorded == expected, operation
    assert result.stdout.strip() == result.stderr == ""


@pytest.mark.parametrize("operation", ["cascade", "cascade-without-sources", "prune"])
def test_unavailable_schedule_reads_still_remove_deleted_instances(tmp_path: Path, operation: str) -> None:
    calls = tmp_path / "calls"
    sources = (
        []
        if operation == "cascade-without-sources"
        else [Source("example-gone-example-tenant", "source-gone", "definition")]
    )
    plan = plan_row("example", "main", "example-secret")
    definitions = listing([Definition("example", "definition")])
    result = bash(
        f"""
        set -euo pipefail
        source '{ROOT}/lib/reconcile.sh'
        _CONNECTOR_SCHEDULES_STATUS=unavailable
        _RECONCILE_CHANGED=0
        log_line() {{ :; }}
        reconcile__log() {{ :; }}
        reconcile_compute_tenant() {{ printf 'example-tenant'; }}
        ab_workspace_id() {{ printf 'workspace'; }}
        ab_list_connections() {{ printf '[]'; }}
        ab_list_definitions() {{ printf '%s' {shlex.quote(definitions)}; }}
        ab_list_sources() {{ printf '%s' {shlex.quote(listing(sources))}; }}
        ab_delete_source() {{ printf 'source %s\n' "$1" >> '{calls}'; }}
        disc_load_descriptors() {{ printf '%s' {shlex.quote(plan)}; }}
        kubectl() {{
          [[ "$*" == *' delete '* ]] || return 1
          printf '%s\n' "${{4#cronworkflow.argoproj.io/}}" >> '{calls}'
          printf 'deleted\n'
        }}
        if [[ '{operation}' == prune ]]; then
          reconcile_prune_removed_instances {shlex.quote(plan)}
        else
          reconcile_cascade_delete example
        fi
        """,
        tmp_path,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    recorded = calls.read_text().splitlines() if calls.exists() else []
    if operation == "cascade-without-sources":
        assert recorded == ["example-example-tenant-sync", "example-example-sync"], recorded
    elif operation == "cascade":
        assert recorded == [
            "source source-gone",
            "example-example-tenant-sync",
            "example-example-sync",
            "example-gone-example-sync",
            "example-example-tenant-sync",
            "example-example-sync",
        ], recorded
    else:
        assert recorded == ["source source-gone", "example-gone-example-sync"], recorded


def test_renderer_accepts_api_and_sweep_preserves_it() -> None:
    result = _render("0 3 * * *", origin="api")
    assert result.returncode == 0, result.stderr
    assert "insight.cyberfabric.com/schedule-origin: api" in result.stdout
    assert read_schedule({"schedules": ["0 3 * * *"], "schedule_origin": "api"}).origin == "api"


@pytest.mark.parametrize(
    ("has_request", "annotation", "expected"),
    [
        (True, "0 1 * * *", "annotation\n0 1 * * *"),
        (True, "", "descriptor\n0 2 * * *"),
        (False, "0 1 * * *", "annotation\n0 1 * * *"),
    ],
)
def test_deployment_fallback_is_resolved_beside_the_request(
    tmp_path: Path, has_request: bool, annotation: str, expected: str
) -> None:
    directory = tmp_path / "area" / "example"
    directory.mkdir(parents=True)
    (directory / "descriptor.yaml").write_text("schedule: '0 2 * * *'\n")
    result = bash(
        f"""
        set -euo pipefail
        source '{ROOT}/lib/connector-naming.sh'
        _CONNECTOR_SCHEDULES_STATUS=ok
        _CONNECTOR_SCHEDULES_RESULT={shlex.quote(json.dumps(GOOD if has_request else {"schedules": []}))}
        kubectl() {{ printf '%s' {shlex.quote(annotation)}; }}
        deployment="$(reconcile_resolve_deployment_schedule example example-secret)"
        reconcile_resolve_schedule example example-secret main "$deployment"
        printf '\nFALLBACK\n%s' "$deployment"
    """,
        tmp_path,
    )
    assert result.returncode == 0, result.stderr
    effective, fallback = result.stdout.split("\nFALLBACK\n")
    assert fallback == expected
    assert effective == ("api\n0 3 * * *\n0 15 * * *" if has_request else expected)


@pytest.mark.parametrize("failure", ["secret", "descriptor"])
def test_deployment_read_failure_never_selects_defaults(tmp_path: Path, failure: str) -> None:
    directory = tmp_path / "area" / "example"
    directory.mkdir(parents=True)
    (directory / "descriptor.yaml").write_text("schedule: [invalid yaml")
    result = bash(
        f"""
        source '{ROOT}/lib/connector-naming.sh'
        _CONNECTOR_SCHEDULES_STATUS=ok
        _CONNECTOR_SCHEDULES_RESULT={shlex.quote(json.dumps(GOOD))}
        kubectl() {{ {"return 1" if failure == "secret" else "return 0"}; }}
        reconcile_resolve_schedule example example-secret main
    """,
        tmp_path,
    )
    assert result.returncode != 0
    assert result.stdout == ""
