from __future__ import annotations

import json
import shlex
from pathlib import Path

import pytest
import yaml
from test_connector_schedule_client import GOOD
from test_connector_schedule_runs import ROOT
from test_cronworkflow_error_logging import run_caller
from test_render_cronworkflow import _renderer_args, _run


@pytest.mark.parametrize("origin", ["annotation", "descriptor", "default", "api"])
@pytest.mark.parametrize("crons", [["0 4 * * *"], ["*/5 * * * *", "0 9,15,21 28-31 * *"]])
def test_deployment_annotation_round_trips_as_a_quoted_yaml_scalar(origin: str, crons: list[str]) -> None:
    effective = ["0 6 * * *"] if origin == "api" else crons
    argv = _renderer_args("\n".join(effective), origin)
    fallback_origin = "descriptor" if origin == "api" else origin
    argv.extend(["--deployment-schedule", "\n".join(crons), "--deployment-schedule-origin", fallback_origin])
    result = _run(argv)

    assert result.returncode == 0, result.stderr
    document = yaml.safe_load(result.stdout)
    key = "insight.cyberfabric.com/deployment-schedule"
    value = document["metadata"]["annotations"][key]
    assert isinstance(value, str)
    assert json.loads(value) == {"origin": fallback_origin, "crons": crons}
    line = next(line for line in result.stdout.splitlines() if key + ":" in line)
    assert line.split(": ", 1)[1].startswith('"')
    assert document["spec"]["schedules"] == effective


def test_api_origin_requires_an_independent_deployment_schedule() -> None:
    result = _run(_renderer_args("0 6 * * *", "api"))
    assert result.returncode == 2
    assert result.stdout == ""


@pytest.mark.parametrize("caller", ["reconcile", "adopt"])
@pytest.mark.parametrize("origin", ["api", "annotation", "descriptor", "default"])
def test_callers_record_the_deployment_fallback_when_saved_schedules_win(
    caller: str, origin: str, tmp_path: Path
) -> None:
    manifest = tmp_path / "manifest.yaml"
    directory = tmp_path / "area" / "example-connector"
    directory.mkdir(parents=True)
    descriptor = "name: example-connector\n"
    if origin != "default":
        descriptor += "schedule: ['0 2 * * *', '0 14 * * *']\n"
    (directory / "descriptor.yaml").write_text(descriptor)
    snapshot = {"schedules": []}
    if origin == "api":
        snapshot["schedules"] = [{**GOOD["schedules"][0], "connector": "example-connector"}]
    annotation = "0 1 * * *\n0 13 * * *" if origin == "annotation" else ""
    overrides = f"""
    unset -f reconcile_resolve_schedule reconcile_resolve_deployment_schedule
    source '{ROOT}/lib/connector-naming.sh'
    source '{ROOT}/lib/argo.sh'
    disc_instances_of() {{ printf 'main\texample-secret\thash\n'; }}
    export ARGO_SERVICE_ACCOUNT=example-reconcile
    _CONNECTOR_SCHEDULES_STATUS=ok
    _CONNECTOR_SCHEDULES_RESULT={shlex.quote(json.dumps(snapshot))}
    kubectl() {{ printf '%s' {shlex.quote(annotation)}; }}
    argo_apply_cronworkflow() {{ argo_render_cronworkflow "$@" > '{manifest}'; }}
    argo_delete_former_tenant_cronworkflows() {{ :; }}
    """
    result = run_caller(caller, tmp_path, overrides)
    assert result.returncode == 0, result.stdout + result.stderr
    document = yaml.safe_load(manifest.read_text())
    annotations = document["metadata"]["annotations"]
    fallback_origin = "descriptor" if origin == "api" else origin
    fallback_crons = {
        "annotation": ["0 1 * * *", "0 13 * * *"],
        "descriptor": ["0 2 * * *", "0 14 * * *"],
        "default": ["0 0 * * *"],
    }[fallback_origin]
    assert json.loads(annotations["insight.cyberfabric.com/deployment-schedule"]) == {
        "origin": fallback_origin,
        "crons": fallback_crons,
    }
    assert annotations["insight.cyberfabric.com/schedule-origin"] == origin
    assert document["spec"]["schedules"] == (GOOD["schedules"][0]["crons"] if origin == "api" else fallback_crons)
