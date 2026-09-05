"""The identity lane: who asks, and how a connector's bronze reaches identity.

Every caller here is a real stand persona chosen for the expectation a test states.
The admin operator makes corrections and sees nobody; the lead the suite grafts its
people beneath sees them and nothing further; a service principal, obtained at the
authenticator's token endpoint, is the only caller identity's internal routes serve.
"""

from __future__ import annotations

import uuid

import pytest
from insight_datapath.ch_seeder import CHSeeder
from insight_datapath.connector_path import ConnectorPath
from insight_datapath.dbt_runner import DbtRunner
from insight_datapath.records import SCHEMAS_DIR
from insight_stand.api import ApiClient
from insight_stand.manifest import Manifest
from insight_stand.personas import PersonaSession
from insight_stand.service_token import default_identity_url, open_service_session


@pytest.fixture(scope="session")
def connector_path(ch_seeder: CHSeeder, dbt_runner: DbtRunner) -> ConnectorPath:
    return ConnectorPath(ch_seeder, dbt_runner, schemas_dir=SCHEMAS_DIR)


@pytest.fixture(scope="session")
def substitutions(stand_manifest: Manifest, caller_session: PersonaSession) -> dict[str, str]:
    """What a template leaves to the instance: the tenant, and the lead people report to."""
    return {
        "tenant": stand_manifest.tenant,
        "supervisor_email": caller_session.email,
        "supervisor_name": caller_session.person.display_name,
    }


@pytest.fixture(scope="session")
def operator(operator_session: PersonaSession) -> ApiClient:
    """The admin operator: every correction verb opens to them, no data does."""
    return operator_session.client


@pytest.fixture(scope="session")
def lead(caller_session: PersonaSession) -> ApiClient:
    """The lead a test's people report to, reading their own team."""
    return caller_session.client


@pytest.fixture(scope="session")
def service_client(stand_manifest: Manifest) -> ApiClient:
    """A service principal at identity's own listener, the way the authenticator calls.

    The gateway is a browser BFF and refuses a bearer-only caller, so the internal
    routes have no edge address; a stand whose token endpoint this runner cannot
    reach has no service principal to offer, and the test that needs one skips.
    """
    if not stand_manifest.capabilities.has("service_principals"):
        pytest.skip("this stand offers no service principal to obtain")
    return ApiClient(
        base_url=default_identity_url(),
        session=open_service_session(stand_manifest.tenant),
        edge_fronted=False,
    )


@pytest.fixture
def run_tag() -> str:
    """Makes a test's accounts unique across runs: the journal is append-only."""
    return uuid.uuid4().hex[:10]
