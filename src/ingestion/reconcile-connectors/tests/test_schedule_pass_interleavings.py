from __future__ import annotations

import json
import shlex
from pathlib import Path

import pytest
from reconcile_inputs import Definition, Source, listing
from test_connector_schedule_runs import ROOT, bash
from test_cronworkflow_error_logging import STUBS

FIXTURE = Path(__file__).with_name("fixtures") / "connector-schedules.json"


@pytest.mark.parametrize(
    ("connector", "source", "expected"),
    [
        ("example-connector", "main-east", "api\n0 3 * * *\n0 15 * * *"),
        ("example-connector", "second-west", "api\n30 6 * * 1-5"),
        ("example-other", "main-east", "api\n0 9 * * *"),
        ("example-other", "cleared-row", "annotation\n0 1 * * *"),
    ],
)
def test_shared_handler_fixture_wins_over_annotation_and_descriptor(
    tmp_path: Path, connector: str, source: str, expected: str
) -> None:
    directory = tmp_path / "area" / connector
    directory.mkdir(parents=True)
    (directory / "descriptor.yaml").write_text("schedule: '0 2 * * *'\n")
    result = bash(
        f"""
        set -euo pipefail
        source '{ROOT}/lib/connector-naming.sh'
        _CONNECTOR_SCHEDULES_STATUS=ok
        _CONNECTOR_SCHEDULES_RESULT="$(cat '{FIXTURE}')"
        kubectl() {{ printf '0 1 * * *'; }}
        reconcile_resolve_schedule {connector} example-secret {source}
        """,
        tmp_path,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    assert result.stdout == expected


@pytest.mark.parametrize("second", ["changed", "annotation", "descriptor", "default", "failed", "deleted", "unknown"])
def test_each_pass_uses_its_read_and_cleanup_survives_schedule_changes(tmp_path: Path, second: str) -> None:
    endpoint = tmp_path / "endpoint.json"
    initial = json.loads(FIXTURE.read_bytes())
    initial["schedules"] = [initial["schedules"][0]]
    initial["schedules"][0]["source_id"] = "main"
    endpoint.write_text(json.dumps(initial))
    following = json.loads(json.dumps(initial))
    if second == "changed":
        following["schedules"][0]["crons"] = ["0 8 * * *"]
    elif second in {"annotation", "descriptor", "default"}:
        following["schedules"] = []
    elif second == "unknown":
        following["schedules"].append({**following["schedules"][0], "source_id": "not-configured"})
    following_path = tmp_path / "following.json"
    following_path.write_text("invalid JSON" if second == "failed" else json.dumps(following))
    client = tmp_path / "client"
    client.mkdir()
    (client / "read_connector_schedules.py").write_text(
        "import json, sys\nfrom dataclasses import asdict\n"
        f"sys.path.insert(0, {str(ROOT / 'python')!r})\n"
        "from read_connector_schedules import ScheduleReadError, validate_response\n"
        "try:\n"
        f"    schedules = validate_response(json.load(open({str(endpoint)!r})))\n"
        "    print(json.dumps({'status':'ok', 'schedules':[asdict(s) for s in schedules]}))\n"
        "except (ValueError, ScheduleReadError):\n"
        "    print(json.dumps({'status':'unavailable','reason':'synthetic read failure'}))\n"
    )
    directory = tmp_path / "area" / "example-connector"
    directory.mkdir(parents=True)
    (directory / "descriptor.yaml").write_text("name: example-connector\nschedule: '0 2 * * *'\n")
    plan = f"example-connector\t{directory}\t1\tnocode\t\t\tbronze_example\tmain\texample-secret\thash"
    removed = "" if second != "deleted" else "example-connector\tdir\t1\tnocode\t\t\tbronze_example\t\t\t"
    definitions = listing([Definition("example-connector", "definition")])
    sources_list = [Source("example-connector-main-example-tenant", "source", "definition")]
    if second == "failed":
        sources_list.append(Source("example-connector-gone-example-tenant", "source-gone", "definition"))
    sources = listing(sources_list)
    calls = tmp_path / "calls"
    state = tmp_path / "schedule"
    result = bash(
        f"""
        set -uo pipefail
        source '{ROOT}/lib/reconcile.sh'
        {STUBS}
        unset -f reconcile_resolve_schedule
        source '{ROOT}/lib/connector-naming.sh'
        _CONNECTOR_SCHEDULES_PY_DIR='{client}'
        pass=1
        log_init() {{ :; }}
        log_close() {{ :; }}
        log_run_summary() {{ :; }}
        log_line() {{ :; }}
        reconcile__log() {{ :; }}
        reconcile_compute_tenant() {{ printf 'example-tenant'; }}
        disc_load_instances() {{
          if [[ "$pass" == 2 && '{second}' == deleted ]]; then
            printf '%s\n' {shlex.quote(removed)}
          else
            printf '%s\n' {shlex.quote(plan)}
          fi
          cp '{following_path}' '{endpoint}'
        }}
        disc_load_descriptors() {{ printf '%s\n' {shlex.quote(plan)}; }}
        reconcile_definitions() {{ printf 'noop\tnone\tdefinition\n'; }}
        ab_workspace_id() {{ printf 'workspace'; }}
        ab_list_definitions() {{ printf '%s' {shlex.quote(definitions)}; }}
        ab_list_sources() {{ printf '%s' {shlex.quote(sources)}; }}
        ab_list_connections() {{ printf '[]'; }}
        ab_delete_source() {{ printf 'remove-source\n' >> '{calls}'; }}
        argo_apply_cronworkflow() {{
          printf '%s|%s|%s\n' "$1" "$5" "$7" >> '{calls}'
          printf '%s' "$3" > '{state}'
        }}
        argo_delete_former_tenant_cronworkflows() {{ :; }}
        reconcile_gc_orphans() {{ :; }}
        sweep_run() {{ :; }}
        kubectl() {{
          if [[ "$*" == *' delete '* ]]; then
            printf 'delete %s\n' "$4" >> '{calls}'
            if [[ '{second}' == deleted ]]; then rm -f '{state}'; fi
          elif [[ "$*" == *' -o json' ]]; then
            printf '%s' '{{"stringData": {{}}}}'
          elif [[ "$pass" == 1 || '{second}' == annotation ]]; then
            printf '0 1 * * *'
          fi
        }}
        reconcile_run 0 1
        first_rc=$?
        cp '{state}' '{tmp_path / "first"}'
        if [[ '{second}' == default ]]; then
          printf 'name: example-connector\n' > '{directory / "descriptor.yaml"}'
        fi
        printf 'pass-two\n' >> '{calls}'
        pass=2
        reconcile_run 0 1
        second_rc=$?
        printf '%s %s\n' "$first_rc" "$second_rc"
        """,
        tmp_path,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    assert result.stdout.strip() == ("0 2" if second == "failed" else "0 0"), result.stdout + result.stderr
    assert (tmp_path / "first").read_text() == "0 3 * * *\n0 15 * * *"
    expected = {
        "changed": "0 8 * * *",
        "annotation": "0 1 * * *",
        "descriptor": "0 2 * * *",
        "default": "0 0 * * *",
    }.get(second, "0 3 * * *\n0 15 * * *")
    if second == "deleted":
        assert not state.exists()
    else:
        assert state.read_text() == expected
    recorded = calls.read_text().splitlines()
    applies = [line for line in recorded if line.startswith("example-connector|")]
    assert len(applies) == (1 if second in {"failed", "deleted"} else 2), recorded
    assert all("not-configured" not in line for line in recorded), recorded
    if second in {"deleted", "failed"}:
        second_calls = recorded[recorded.index("pass-two") + 1 :]
        assert "remove-source" in second_calls, recorded
        assert any(line.startswith("delete cronworkflow.argoproj.io/") for line in second_calls), recorded
