"""The instance a data-path run reads and writes, and the caller it asks as.

This tree states its own session, deliberately: `tests/stand/conftest.py` governs only
its own directory, and the two suites want opposite things from an instance. The stand
suite reads a seeded stand and writes nothing; this one seeds the warehouse and clears
it between specs, so it must never be pointed at a stand another suite is using.
"""

from __future__ import annotations

import os
from collections.abc import Iterator
from pathlib import Path

import pytest
from insight_datapath.bindings import Bindings
from insight_datapath.caller import StandCaller
from insight_datapath.ch_seeder import CHSeeder
from insight_datapath.dbt_runner import DbtRunner
from insight_datapath.enrich import EnrichRunner
from insight_datapath.instance import InstanceConfig, resolve_instance
from insight_datapath.reset import refuse_a_seeded_warehouse, session_floor
from insight_datapath.schema import apply_all, restart_analytics
from insight_datapath.subjects import Subjects
from insight_stand.manifest import Manifest, load_manifest
from insight_stand.personas import ADMIN_OPERATOR_FIXTURE, PersonaSession, open_session
from insight_stand.stand import resolve_endpoint

REPO_ROOT = Path(__file__).resolve().parents[2]

#: Every spec's people are grafted beneath this persona, who then asks for their metrics.
CALLER_FIXTURE = "dev_lead"


_ENV_FILE_STEM = ".env.compose.test-stand"


def _env_file() -> Path:
    return Path(os.environ.get("INSIGHT_STAND_ENV_FILE", _ENV_FILE_STEM))


def _instance_name() -> str:
    """The compose project the instance runs under, from its env file's name.

    `INSIGHT_STAND_ENV_FILE` is a public override, so a file dev-compose.sh did not
    write is refused here rather than reaching `docker compose --project-name` as a
    name nothing answers to.
    """
    name = _env_file().name
    if not name.startswith(_ENV_FILE_STEM):
        raise RuntimeError(
            f"INSIGHT_STAND_ENV_FILE names {name!r}, which is not a test-stand env file: "
            f"the compose project is read from a {_ENV_FILE_STEM}[-<instance>] name."
        )
    suffix = name.removeprefix(_ENV_FILE_STEM)
    return f"insight-{suffix.lstrip('-')}" if suffix.strip("-") else "insight"


@pytest.fixture(scope="session")
def instance_cfg() -> InstanceConfig:
    """Which instance's databases this run seeds and clears."""
    return resolve_instance()


@pytest.fixture(scope="session")
def stand_manifest() -> Manifest:
    return load_manifest()


@pytest.fixture(scope="session")
def tenant(stand_manifest: Manifest) -> str:
    """The tenant this instance serves, which every seeded row belongs to."""
    return stand_manifest.tenant


@pytest.fixture(scope="session")
def caller_session(stand_manifest: Manifest) -> PersonaSession:
    """The seeded persona whose subtree a spec's people are grafted into."""
    return open_session(CALLER_FIXTURE, stand_manifest, resolve_endpoint().base_url)


@pytest.fixture(scope="session")
def caller(caller_session: PersonaSession) -> StandCaller:
    return StandCaller(caller_session.client)


@pytest.fixture(scope="session")
def operator_session(stand_manifest: Manifest) -> PersonaSession:
    """The seeded persona that may make an operator decision about an account.

    A binding is admin-gated, and the caller a spec reads as is an ordinary lead.
    """
    return open_session(ADMIN_OPERATOR_FIXTURE, stand_manifest, resolve_endpoint().base_url)


@pytest.fixture(scope="session")
def bindings(instance_cfg: InstanceConfig, operator_session: PersonaSession) -> Bindings:
    return Bindings(instance_cfg, client=operator_session.client)


@pytest.fixture(scope="session")
def ch_seeder(instance_cfg: InstanceConfig) -> CHSeeder:
    """One seeder for the session: its ledger must outlive a spec to clear it."""
    return CHSeeder(instance_cfg)


@pytest.fixture(scope="session")
def warehouse_is_ours(stand_manifest: Manifest) -> None:
    """Refuse an instance whose own seed shares the relations a spec builds."""
    refuse_a_seeded_warehouse(stand_manifest.seeded)


@pytest.fixture(scope="session")
def warehouse_schema(instance_cfg: InstanceConfig, warehouse_is_ours: None) -> int:
    """Give the instance the schema a connector's first sync would have left.

    A stand raised with `test-stand minimal` carries identity and nothing else, so
    the databases a spec seeds have to be created before anything can write them.
    """
    applied = apply_all(instance_cfg, repo_root=REPO_ROOT)
    restart_analytics(repo_root=REPO_ROOT, project=_instance_name(), env_file=_env_file())
    return applied


@pytest.fixture(scope="session")
def warehouse_floor(instance_cfg: InstanceConfig, warehouse_schema: int) -> int:
    """Empty every fixture-data relation before the first spec seeds."""
    return session_floor(instance_cfg)


@pytest.fixture(scope="session")
def dbt_runner(instance_cfg: InstanceConfig, warehouse_floor: int) -> Iterator[DbtRunner]:
    """dbt against this instance, with every non-gold model materialized once.

    The closure build is what makes relation existence a session constant: a spec
    builds only its own slice, and a compile-time probe for a relation another
    connector owns must answer the same however few specs have run.
    """
    runner = DbtRunner(
        instance_cfg,
        project_dir=REPO_ROOT / "src/ingestion/dbt",
        target_dir=REPO_ROOT / "tests/datapath/.dbt",
    )
    runner.setup()
    runner.build_closure()
    yield runner
    runner.cleanup()


@pytest.fixture(scope="session")
def enrich_runner(instance_cfg: InstanceConfig) -> EnrichRunner:
    runner = EnrichRunner(
        instance_cfg,
        repo_root=REPO_ROOT,
        project=_instance_name(),
        env_file=_env_file(),
    )
    runner.pull_images()
    return runner


@pytest.fixture(scope="session")
def subjects(instance_cfg: InstanceConfig, tenant: str) -> Subjects:
    """persons-seed mints under the tenant the assertions read, or it mints out of sight."""
    return Subjects(
        instance_cfg,
        repo_root=REPO_ROOT,
        project=_instance_name(),
        env_file=_env_file(),
        tenant_id=tenant,
    )
