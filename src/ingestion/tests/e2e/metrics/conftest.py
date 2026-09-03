"""Module-scoped spec runs for the ported metric specs.

A ported spec is a module that names its fixture file in `SPEC` and asserts one
case per test. Its data path is built once per module; each test then reads through
`spec.call(...)` and is held to the same completeness rule the YAML engine enforced.
"""

from __future__ import annotations

from collections.abc import Iterator
from pathlib import Path

import pytest
from lib.analytics import AnalyticsProcess
from lib.ch_seeder import CHSeeder
from lib.config import SessionConfig
from lib.dbt_runner import DbtRunner
from lib.enrich import EnrichRunner
from lib.fixture_loader import load
from lib.identity_stub import IdentityStub
from lib.metric_expect import Ledger
from lib.spec_runner import SpecRun, run_spec
from lib.worker import WorkerContext

METRICS_ROOT = Path(__file__).parent
ARTIFACTS = METRICS_ROOT.parent / ".artifacts"

LEDGER = Ledger()


@pytest.fixture(scope="session", autouse=True)
def _write_ledger() -> Iterator[None]:
    """The coverage gate reads what the ported specs asserted from this file."""
    yield
    LEDGER.write(ARTIFACTS / "metric_assertions.json")


@pytest.fixture(scope="module")
def spec(
    request: pytest.FixtureRequest,
    ch_migrations_applied: SessionConfig,
    ch_seeder: CHSeeder,
    dbt_runner: DbtRunner,
    enrich_runner: EnrichRunner,
    analytics: AnalyticsProcess,
    identity_stub: IdentityStub,
    worker_ctx: WorkerContext,
) -> SpecRun:
    name = getattr(request.module, "SPEC", None)
    if not name:
        raise pytest.UsageError(f"{request.module.__name__}: a ported spec module must set SPEC = '<fixture name>'")
    loaded = load(METRICS_ROOT / f"{name}.test.yaml")
    if loaded.skip:
        pytest.skip(loaded.skip)
    return run_spec(
        loaded,
        cfg=ch_migrations_applied,
        ch_seeder=ch_seeder,
        dbt_runner=dbt_runner,
        enrich_runner=enrich_runner,
        analytics=analytics,
        identity_stub=identity_stub,
        worker_ctx=worker_ctx,
        ledger=LEDGER,
    )


@pytest.fixture(autouse=True)
def _complete_every_row(request: pytest.FixtureRequest) -> Iterator[None]:
    """Applies only to tests that read through `spec`; checks their rows at the end."""
    if "spec" not in request.fixturenames:
        yield
        return
    run: SpecRun = request.getfixturevalue("spec")
    run.begin(request.node.name)
    yield
    run.end()
