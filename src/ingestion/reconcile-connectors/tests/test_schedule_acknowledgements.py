from __future__ import annotations

import json
import sys
from copy import deepcopy
from pathlib import Path
from unittest.mock import MagicMock

import acknowledge_connector_schedule as writer
import pytest
from test_connector_schedule_client import ENV

PLAN = {
    "connector": "example",
    "source_id": "main",
    "name": "example-main-tenant-sync",
    "namespace": "example-namespace",
    "tenant": "tenant",
    "revision": 7,
    "crons": ["0 3 * * *"],
    "origin": "api",
    "deployment_crons": ["0 1 * * *"],
    "deployment_origin": "annotation",
}
OBSERVED = {
    "metadata": {
        "name": PLAN["name"],
        "namespace": PLAN["namespace"],
        "labels": {
            writer.PREFIX + "connector": "example",
            writer.PREFIX + "tenant": "tenant",
            "app.kubernetes.io/component": "connector-sync",
        },
        "annotations": {writer.PREFIX + "schedule-origin": "api"},
    },
    "spec": {
        "schedules": PLAN["crons"],
        "timezone": "UTC",
        "suspend": True,
        "workflowSpec": {"arguments": {"parameters": [{"name": "insight_source_id", "value": "main"}]}},
    },
}


def inserted(cursor: MagicMock) -> tuple[dict, str]:
    sql, values = cursor.execute.call_args.args
    columns = sql.split("(", 1)[1].split(")", 1)[0].split(", ")
    return dict(zip(columns, values, strict=True)), sql.split(" ON DUPLICATE KEY UPDATE ")[1]


@pytest.mark.parametrize("revision", [0, 7, 8], ids=["inherited-without-request", "override", "clear"])
def test_success_records_all_observed_and_deployment_fields(database: MagicMock, revision: int) -> None:
    plan = {**PLAN, "revision": revision}
    if revision != 7:
        plan.update(crons=plan["deployment_crons"], origin=plan["deployment_origin"])
    observed = deepcopy(OBSERVED)
    observed["spec"]["schedules"] = plan["crons"]
    observed["metadata"]["annotations"][writer.PREFIX + "schedule-origin"] = plan["origin"]
    assert writer.acknowledge(ENV, {"plan": plan, "observed": observed})
    row, updates = inserted(database)
    assert row == {
        "connector": "example",
        "source_id": "main",
        "applied_revision": revision,
        "applied_crons": json.dumps(plan["crons"]),
        "applied_timezone": "UTC",
        "applied_origin": plan["origin"],
        "suspended": True,
        "deployment_crons": json.dumps(PLAN["deployment_crons"]),
        "deployment_origin": "annotation",
        "observed_at": row["attempted_at"],
        "attempted_at": row["attempted_at"],
        "attempted_revision": revision,
        "last_error": None,
    }
    assert all(f"{column}=VALUES({column})" in updates for column in row if column not in {"connector", "source_id"})


@pytest.mark.parametrize("error", ["schedule apply failed", "schedule readback failed", "synthetic-secret" * 600])
def test_failed_attempt_preserves_every_success_field(database: MagicMock, error: str) -> None:
    assert not writer.acknowledge(ENV, {"plan": PLAN, "error": error})
    row, updates = inserted(database)
    assert row["last_error"] in {"schedule apply failed", "schedule readback failed"}
    assert (
        updates
        == "attempted_revision=VALUES(attempted_revision), attempted_at=VALUES(attempted_at), last_error=VALUES(last_error)"
    )
    assert "applied_revision" not in row and "observed_at" not in row


@pytest.mark.parametrize(
    "field", ["name", "namespace", "connector", "tenant", "source_id", "crons", "timezone", "origin"]
)
def test_readback_must_match_the_applied_identity_and_schedule(database: MagicMock, field: str) -> None:
    observed = deepcopy(OBSERVED)
    if field in {"name", "namespace"}:
        observed["metadata"][field] = "wrong"
    elif field in {"connector", "tenant"}:
        observed["metadata"]["labels"][writer.PREFIX + field] = "wrong"
    elif field == "source_id":
        observed["spec"]["workflowSpec"]["arguments"]["parameters"][0]["value"] = "wrong"
    elif field == "origin":
        observed["metadata"]["annotations"][writer.PREFIX + "schedule-origin"] = "default"
    else:
        observed["spec"]["schedules" if field == "crons" else field] = "wrong"
    assert not writer.acknowledge(ENV, {"plan": PLAN, "observed": observed})
    row, updates = inserted(database)
    assert row["last_error"] == "schedule readback mismatch"
    assert "applied_revision" not in updates


def test_acknowledgement_uses_snapshot_revision_even_after_a_new_save(database: MagicMock) -> None:
    database.__iter__.side_effect = AssertionError("must not reread revision 8")
    assert writer.acknowledge(ENV, {"plan": PLAN, "observed": OBSERVED})
    row, _ = inserted(database)
    assert row["applied_revision"] == 7 < 8
    assert database.execute.call_count == 1


def test_write_failure_is_propagated(database: MagicMock) -> None:
    database.execute.side_effect = RuntimeError("synthetic-secret")
    with pytest.raises(RuntimeError):
        writer.acknowledge(ENV, {"plan": PLAN, "observed": OBSERVED})


@pytest.mark.parametrize("caller", ["reconcile", "adopt"])
@pytest.mark.parametrize("write_fails", [False, True])
def test_acknowledgement_write_failure_fails_the_application(caller: str, write_fails: bool, tmp_path: Path) -> None:
    from test_cronworkflow_error_logging import run_caller

    client = tmp_path / "client"
    client.mkdir()
    captured = tmp_path / "attempt.json"
    (client / "acknowledge_connector_schedule.py").write_text(
        "import sys\nfrom pathlib import Path\n"
        f"Path({str(captured)!r}).write_text(sys.stdin.read())\nraise SystemExit({int(write_fails)})\n"
    )
    kubectl = tmp_path / "kubectl"
    kubectl.write_text(f'#!{sys.executable}\nprint(\'{{"metadata":{{}}, "spec":{{}}}}\')\n')
    kubectl.chmod(0o700)
    overrides = f"""
        unset -f connector_schedules_acknowledge
        source '{{ROOT}}/lib/connector-schedules.sh'
        _CONNECTOR_SCHEDULES_PY_DIR='{client}'
        _CONNECTOR_SCHEDULES_STATUS=ok
        _CONNECTOR_SCHEDULES_RESULT='{{"schedules":[{{"connector":"example-connector","source_id":"main","revision":7,"crons":["0 3 * * *"]}}]}}'
        reconcile_resolve_deployment_schedule() {{ printf 'annotation\n0 1 * * *'; }}
        argo_apply_cronworkflow() {{ return 0; }}
        argo_delete_former_tenant_cronworkflows() {{ return 0; }}
        export PATH='{tmp_path}':"$PATH"
    """
    from test_cronworkflow_error_logging import ROOT

    result = run_caller(caller, tmp_path, overrides.replace("{ROOT}", str(ROOT)))
    assert result.returncode == int(write_fails), result.stdout + result.stderr
    payload = json.loads(captured.read_text())
    assert payload["plan"]["revision"] == 7
    assert payload["plan"]["deployment_crons"] == ["0 1 * * *"]
    assert payload["plan"]["deployment_origin"] == "annotation"
    assert payload["observed"] == {"metadata": {}, "spec": {}}


@pytest.mark.parametrize(
    "observed",
    [{}, {"metadata": None}, {"spec": None}, {"metadata": OBSERVED["metadata"], "spec": {"workflowSpec": None}}],
)
def test_malformed_readback_records_a_failed_attempt(database: MagicMock, observed: dict) -> None:
    assert not writer.acknowledge(ENV, {"plan": PLAN, "observed": observed})
    row, updates = inserted(database)
    assert row["last_error"] == "schedule readback mismatch"
    assert "observed_at" not in updates
