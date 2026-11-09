"""Prepare isolated scenario batches and enforce metric assertion completeness.

A spec module may also name another spec in `NEIGHBOUR`, seeded and minted under a
second tenant beside it, so the module's reads prove that tenant stays out of sight.
"""

from __future__ import annotations

from collections.abc import Iterator
from pathlib import Path

import pytest
from insight_datapath.bindings import Bindings
from insight_datapath.caller import StandCaller
from insight_datapath.ch_seeder import CHSeeder
from insight_datapath.dataset_batches import content_tokens, plan_batches
from insight_datapath.dbt_runner import DbtRunner
from insight_datapath.fixture_loader import load
from insight_datapath.instance import InstanceConfig
from insight_datapath.metric_definitions import collect
from insight_datapath.metric_expect import Ledger
from insight_datapath.shared_dataset import Scenario, combine, namespace
from insight_datapath.spec_runner import NEIGHBOUR_TENANT, Neighbour, SpecRun, run_spec
from insight_datapath.subjects import Subjects
from insight_stand.manifest import Manifest
from insight_stand.personas import PersonaSession
from insight_stand.stand import artifact_dir

METRICS_ROOT = Path(__file__).parent
REPO_ROOT = METRICS_ROOT.parents[2]

LEDGER = Ledger()
BATCHES = pytest.StashKey[dict[Path, int]]()


def batch_number(config: pytest.Config, module_path: Path) -> int:
    return config.stash[BATCHES].get(module_path.resolve(), 0)


@pytest.fixture(scope="session")
def shared_readers() -> dict[int, dict[Path, SpecRun]]:
    return {}


def pytest_collection_modifyitems(config: pytest.Config, items: list[pytest.Item]) -> None:
    if config.getoption("numprocesses", default=0):
        raise pytest.UsageError("Datapath tests require one pytest process per isolated stand")
    substitutions = {
        "tenant": "00000000-0000-0000-0000-000000000001",
        "supervisor_email": "lead@example.com",
        "supervisor_name": "Test Lead",
    }
    tokens = {
        str(path): content_tokens(load(path, substitutions=substitutions).bronze)
        for path in sorted((METRICS_ROOT / "git").glob("*.test.yaml"))
    }
    config.stash[BATCHES] = {
        (
            Path(path).parent / f"test_{Path(path).name.removesuffix('.test.yaml')}.py"
        ).resolve(): group + 1
        for path, group in plan_batches(tokens).items()
    }
    items.sort(
        key=lambda item: (
            item.get_closest_marker("shared_dataset") is None,
            batch_number(config, item.path),
        )
    )


@pytest.fixture(scope="module")
def shared_batch(
    request: pytest.FixtureRequest,
    shared_readers: dict[int, dict[Path, SpecRun]],
    ch_seeder: CHSeeder,
    dbt_runner: DbtRunner,
    subjects: Subjects,
    caller: StandCaller,
    caller_session: PersonaSession,
    stand_manifest: Manifest,
    bindings: Bindings,
) -> dict[Path, SpecRun]:
    group = batch_number(request.config, request.node.path)
    if group in shared_readers:
        return shared_readers[group]
    shared_readers.clear()
    scenarios: dict[Path, Scenario] = {}
    for item in request.session.items:
        if batch_number(request.config, item.path) != group:
            continue
        if item.get_closest_marker("shared_dataset") is None:
            continue
        module = item.module
        path = Path(module.__file__).parent / f"{module.SPEC}.test.yaml"
        if path in scenarios:
            continue
        loaded = load(
            path,
            substitutions={
                "tenant": stand_manifest.tenant,
                "supervisor_email": caller_session.email,
                "supervisor_name": caller_session.person.display_name,
            },
        )
        if loaded.skip:
            continue
        scenarios[path] = namespace(
            loaded, key=path.relative_to(METRICS_ROOT).as_posix(), caller_email=caller_session.email
        )
    prepared = run_spec(
        combine(list(scenarios.values())),
        ch_seeder=ch_seeder,
        dbt_runner=dbt_runner,
        subjects=subjects,
        caller=caller,
        caller_email=caller_session.email,
        tenant=stand_manifest.tenant,
        bindings=bindings,
        ledger=LEDGER,
    )
    readers = {path: scenario.reader(prepared) for path, scenario in scenarios.items()}
    shared_readers[group] = readers
    return readers


@pytest.fixture(scope="session", autouse=True)
def _write_coverage_inputs(instance_cfg: InstanceConfig) -> Iterator[None]:
    """The coverage gate reads what this run asserted, and what it was owed, from here."""
    yield
    out = artifact_dir(REPO_ROOT / ".artifacts")
    LEDGER.write(out / "metric_assertions.json")
    collect(instance_cfg, out)


@pytest.fixture(scope="module")
def spec(
    request: pytest.FixtureRequest,
    ch_seeder: CHSeeder,
    dbt_runner: DbtRunner,
    subjects: Subjects,
    caller: StandCaller,
    caller_session: PersonaSession,
    stand_manifest: Manifest,
    bindings: Bindings,
) -> SpecRun:
    name = getattr(request.module, "SPEC", None)
    if not name:
        raise pytest.UsageError(
            f"{request.module.__name__}: a spec module must set SPEC = '<fixture name>'"
        )
    module_dir = Path(request.module.__file__).parent
    substitutions = {
        "tenant": stand_manifest.tenant,
        "supervisor_email": caller_session.email,
        "supervisor_name": caller_session.person.display_name,
    }
    loaded = load(module_dir / f"{name}.test.yaml", substitutions=substitutions)
    if loaded.skip:
        pytest.skip(loaded.skip)
    neighbour = _neighbour(request, module_dir, substitutions, subjects)
    if request.node.get_closest_marker("shared_dataset"):
        if neighbour is not None:
            raise pytest.UsageError(
                f"{request.module.__name__}: a NEIGHBOUR spec seeds its own second tenant "
                "and cannot share a prepared batch"
            )
        prepared = request.getfixturevalue("shared_batch")
        return prepared[loaded.path]
    return run_spec(
        loaded,
        ch_seeder=ch_seeder,
        dbt_runner=dbt_runner,
        subjects=subjects,
        caller=caller,
        caller_email=caller_session.email,
        tenant=stand_manifest.tenant,
        bindings=bindings,
        ledger=LEDGER,
        neighbour=neighbour,
    )


def _neighbour(
    request: pytest.FixtureRequest,
    module_dir: Path,
    substitutions: dict[str, str],
    subjects: Subjects,
) -> Neighbour | None:
    name = getattr(request.module, "NEIGHBOUR", None)
    if not name:
        return None
    loaded = load(
        module_dir / f"{name}.test.yaml",
        substitutions={**substitutions, "tenant": NEIGHBOUR_TENANT},
    )
    if loaded.skip or loaded.identity_accounts or loaded.identity_aliases:
        raise pytest.UsageError(
            f"{request.module.__name__}: NEIGHBOUR names a skipped spec or one with bindings, "
            "which a neighbour does not seed"
        )
    return Neighbour(spec=loaded, subjects=subjects.for_tenant(NEIGHBOUR_TENANT))


@pytest.fixture(autouse=True)
def _complete_every_row(request: pytest.FixtureRequest) -> Iterator[None]:
    """Applies to tests that read through `spec`; checks their rows at the end."""
    if "spec" not in request.fixturenames:
        yield
        return
    run: SpecRun = request.getfixturevalue("spec")
    run.begin(request.node.name)
    yield
    run.end()
