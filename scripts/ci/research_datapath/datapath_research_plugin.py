from __future__ import annotations

import json
import os
import time
from collections.abc import Callable, Iterator
from pathlib import Path
from typing import Any

import httpx
import pytest
from insight_datapath.dbt_runner import DbtRunner
from insight_datapath.fixture_loader import load
from insight_datapath.instance import InstanceConfig
from insight_datapath.metric_definitions import collect
from insight_datapath.metric_expect import Ledger
from insight_datapath.schema import restart_analytics
from insight_datapath.spec_runner import SpecRun, run_spec
from insight_stand.manifest import Manifest
from insight_stand.stand import parse_env_file
from snapshot import capture, duration_values, replay

MODE = os.environ["INSIGHT_RESEARCH_MODE"]
OUT = Path(os.environ["INSIGHT_RESEARCH_OUTPUT"])
ROOT = Path(os.environ["INSIGHT_RESEARCH_SOURCE"])
SNAPSHOT = Path(os.environ["INSIGHT_RESEARCH_SNAPSHOT"])
LEDGER = Ledger()
TIMINGS: list[dict[str, object]] = []
REPLIES: list[object] = []
OUTCOMES: list[tuple[str, str, str]] = []


@pytest.fixture(scope="session", autouse=True)
def _write_coverage_inputs(instance_cfg: InstanceConfig) -> Iterator[None]:
    yield
    LEDGER.write(OUT / "metric_assertions.json")
    collect(instance_cfg, OUT)


@pytest.fixture(scope="session")
def seeded_gold(
    request: pytest.FixtureRequest, instance_cfg: InstanceConfig, stand_manifest: Manifest, warehouse_is_ours: None
) -> None:
    started = time.monotonic()
    replay(instance_cfg, stand_manifest.tenant, SNAPSHOT)
    restart_analytics(
        repo_root=ROOT,
        project=os.environ["INSIGHT_RESEARCH_PROJECT"],
        env_file=Path(os.environ["INSIGHT_STAND_ENV_FILE"]),
    )
    wait_analytics()
    TIMINGS.append({"stage": "gold_replay_and_restart", "seconds": time.monotonic() - started})


def wait_analytics() -> None:
    values = parse_env_file(Path(os.environ["INSIGHT_STAND_ENV_FILE"]))
    port = int(values["ANALYTICS_PORT"])
    deadline = time.monotonic() + 60
    with httpx.Client(timeout=1.0) as client:
        while time.monotonic() < deadline:
            try:
                if client.get(f"http://localhost:{port}/health").status_code == 200:
                    return
            except httpx.TransportError:
                pass
            time.sleep(0.1)
    raise TimeoutError("analytics health did not become ready")


@pytest.fixture(scope="module", params=[1, 2, 3])
def spec(request: pytest.FixtureRequest, instance_cfg: InstanceConfig, stand_manifest: Manifest) -> SpecRun:
    caller_session = request.getfixturevalue("caller_session")
    caller = request.getfixturevalue("caller")
    loaded = load(
        Path(request.module.__file__).with_name("ci_run_duration_min.test.yaml"),
        substitutions={
            "tenant": stand_manifest.tenant,
            "supervisor_email": caller_session.email,
            "supervisor_name": caller_session.person.display_name,
        },
    )
    loaded.bronze["bronze_github.workflow_runs"].append(dict(loaded.bronze["bronze_github.workflow_runs"][0]))
    if MODE == "api":
        request.getfixturevalue("seeded_gold")
        duration_values(instance_cfg, stand_manifest.tenant)
        return SpecRun(
            spec=loaded, caller=caller, tenant=stand_manifest.tenant, to_person_id={}, to_email={}, ledger=LEDGER
        )
    if MODE not in ("baseline", "data"):
        raise ValueError("unknown research mode")
    seeder = request.getfixturevalue("ch_seeder")
    runner = request.getfixturevalue("dbt_runner")
    subjects = request.getfixturevalue("subjects")
    bindings = request.getfixturevalue("bindings")
    started = time.monotonic()
    result = run_spec(
        loaded,
        ch_seeder=seeder,
        dbt_runner=runner,
        subjects=subjects,
        caller=caller,
        caller_email=caller_session.email,
        tenant=stand_manifest.tenant,
        bindings=bindings,
        ledger=LEDGER,
    )
    duration_values(instance_cfg, stand_manifest.tenant)
    TIMINGS.append(
        {"stage": "transformation_with_dbt_tests", "repeat": request.param, "seconds": time.monotonic() - started}
    )
    if MODE == "baseline" and request.param == 3:
        capture(instance_cfg, stand_manifest.tenant, runner.target_dir, SNAPSHOT)
    return result


@pytest.fixture(scope="session", autouse=True)
def record_replies(monkeypatch_session: pytest.MonkeyPatch) -> None:
    from insight_datapath.caller import StandCaller

    original = StandCaller.call_request

    def call(caller: StandCaller, request: dict[str, Any]) -> tuple[int, Any]:
        response = original(caller, request)
        REPLIES.append(response)
        return response

    monkeypatch_session.setattr(StandCaller, "call_request", call)


def measured_dbt(method_name: str) -> Callable[..., object]:
    original = getattr(DbtRunner, method_name)

    def invoke(runner: DbtRunner, *args: object, **kwargs: object) -> object:
        started = time.monotonic()
        result = original(runner, *args, **kwargs)
        row: dict[str, object] = {
            "stage": "dbt_" + method_name,
            "seconds": time.monotonic() - started,
            "selector": args[0] if args else None,
        }
        results = runner.target_dir / "run_results.json"
        if method_name != "setup" and results.exists():
            row["nodes"] = [
                {key: entry[key] for key in ("unique_id", "status", "execution_time")}
                for entry in json.loads(results.read_text())["results"]
            ]
        TIMINGS.append(row)
        return result

    return invoke


@pytest.fixture(scope="session", autouse=True)
def instrument_dbt(monkeypatch_session: pytest.MonkeyPatch) -> None:
    for name in ("setup", "build_closure", "run", "build"):
        monkeypatch_session.setattr(DbtRunner, name, measured_dbt(name))


@pytest.fixture(scope="session")
def monkeypatch_session() -> Iterator[pytest.MonkeyPatch]:
    patch = pytest.MonkeyPatch()
    yield patch
    patch.undo()


@pytest.hookimpl(hookwrapper=True)
def pytest_fixture_setup(fixturedef: pytest.FixtureDef[object], request: pytest.FixtureRequest) -> Iterator[None]:
    started = time.monotonic()
    yield
    if fixturedef.argname in ("warehouse_schema", "warehouse_floor", "dbt_runner", "caller_session"):
        TIMINGS.append({"stage": "fixture_" + fixturedef.argname, "seconds": time.monotonic() - started})


@pytest.hookimpl(hookwrapper=True)
def pytest_runtest_call(item: pytest.Item) -> Iterator[None]:
    started = time.monotonic()
    yield
    TIMINGS.append({"stage": "original_api_assertions", "test": item.nodeid, "seconds": time.monotonic() - started})


def pytest_runtest_logreport(report: pytest.TestReport) -> None:
    OUTCOMES.append((report.nodeid, report.when, report.outcome))


def pytest_sessionfinish(session: pytest.Session, exitstatus: int) -> None:
    (OUT / "observations.json").write_text(
        json.dumps(
            {
                "mode": MODE,
                "exit_code": int(exitstatus),
                "timings": TIMINGS,
                "replies": REPLIES,
                "outcomes": sorted(OUTCOMES),
            },
            indent=2,
        )
        + "\n"
    )
