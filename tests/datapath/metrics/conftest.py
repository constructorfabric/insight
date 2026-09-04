"""One built data path per spec module, and the completeness rule over its tests.

A spec module names its fixture in `SPEC` and asserts one case per test. The path is
built once for the module; each test then reads through `spec.call(...)` and is held to
the rule the retired engine enforced — a selected row owes its view's required fields.
"""

from __future__ import annotations

from collections.abc import Iterator
from pathlib import Path

import pytest
from insight_datapath.caller import StandCaller
from insight_datapath.ch_seeder import CHSeeder
from insight_datapath.dbt_runner import DbtRunner
from insight_datapath.enrich import EnrichRunner
from insight_datapath.fixture_loader import load
from insight_datapath.metric_expect import Ledger
from insight_datapath.spec_runner import SpecRun, run_spec
from insight_datapath.subjects import Subjects
from insight_stand.manifest import Manifest
from insight_stand.personas import PersonaSession
from insight_stand.stand import artifact_dir

METRICS_ROOT = Path(__file__).parent
REPO_ROOT = METRICS_ROOT.parents[2]

LEDGER = Ledger()


@pytest.fixture(scope="session", autouse=True)
def _write_ledger() -> Iterator[None]:
    """The coverage gate reads what this run asserted from here."""
    yield
    LEDGER.write(artifact_dir(REPO_ROOT / ".artifacts") / "metric_assertions.json")


@pytest.fixture(scope="module")
def spec(
    request: pytest.FixtureRequest,
    ch_seeder: CHSeeder,
    dbt_runner: DbtRunner,
    enrich_runner: EnrichRunner,
    subjects: Subjects,
    caller: StandCaller,
    caller_session: PersonaSession,
    stand_manifest: Manifest,
) -> SpecRun:
    name = getattr(request.module, "SPEC", None)
    if not name:
        raise pytest.UsageError(
            f"{request.module.__name__}: a spec module must set SPEC = '<fixture name>'"
        )
    loaded = load(
        METRICS_ROOT / f"{name}.test.yaml",
        substitutions={
            "tenant": stand_manifest.tenant,
            "supervisor_email": caller_session.email,
            "supervisor_name": caller_session.person.display_name,
        },
    )
    if loaded.skip:
        pytest.skip(loaded.skip)
    return run_spec(
        loaded,
        ch_seeder=ch_seeder,
        dbt_runner=dbt_runner,
        enrich_runner=enrich_runner,
        subjects=subjects,
        caller=caller,
        caller_email=caller_session.email,
        ledger=LEDGER,
    )


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
