"""The GitHub-brokered login path, end to end.

`tests/stand/api/identity/test_internal.py` already covers what
`/internal/persons/by-external-id` does with a row that exists: a service
principal resolves it, a person is refused. It says nothing about where that
row comes from — it reads a seeded manifest fixture, not a connector.

These tests cover the other half. They seed the GitHub member roster through
the REAL path (bronze -> github-directory connector dbt models -> the shared
`identity_inputs` union) and then resolve the way the authenticator does, so a
break anywhere in the chain that produces the binding surfaces here rather than
as a 403 at login. The second test pins the casing contract, which no endpoint
test can see because it depends on what the connector wrote.
"""

from __future__ import annotations

import uuid

import pytest
from lib import clickhouse
from lib import identity_seed as seed
from lib.ch_seeder import CHSeeder
from lib.config import SessionConfig
from lib.dbt_runner import DbtRunner
from lib.worker import WorkerContext

pytestmark = [pytest.mark.identity, pytest.mark.mutating]

# No `schemas/bronze_github_directory.org_members.yaml` fixture exists (the
# metrics rig has never needed to seed this table) — declared here from
# `scripts/connectors-ddl/github-directory.sql`.
_GITHUB_DIRECTORY_SCHEMAS = {
    "bronze_github_directory.org_members": {
        "properties": {
            "_airbyte_raw_id": {"type": "string"},
            "_airbyte_extracted_at": {"type": "string", "format": "date-time"},
            "_airbyte_meta": {"type": "string"},
            "_airbyte_generation_id": {"type": "integer"},
            "tenant_id": {"type": ["string", "null"]},
            "source_id": {"type": ["string", "null"]},
            "unique_key": {"type": ["string", "null"]},
            "collected_at": {"type": ["string", "null"]},
            "data_source": {"type": ["string", "null"]},
            "org": {"type": ["string", "null"]},
            "login": {"type": ["string", "null"]},
            "login_normalized": {"type": ["string", "null"]},
            "member_id": {"type": ["integer", "null"]},
            "name": {"type": ["string", "null"]},
            "email": {"type": ["string", "null"]},
            "company": {"type": ["string", "null"]},
            "role": {"type": ["string", "null"]},
            "created_at": {"type": ["string", "null"]},
            "updated_at": {"type": ["string", "null"]},
        }
    }
}


def _org_member(*, run_tag: str, login: str, email: str, display_name: str) -> dict:
    """A minimal `bronze_github_directory.org_members` row, shaped the way the
    connector emits one: `login_normalized` is the lowercased login and the
    entity key follows it.
    """
    normalized = login.lower()
    return {
        "_airbyte_raw_id": str(uuid.uuid4()),
        "_airbyte_extracted_at": "2026-01-05T00:00:00",
        "_airbyte_meta": "{}",
        "_airbyte_generation_id": 0,
        "tenant_id": f"pipeline-tenant-{run_tag}",
        "source_id": f"pipeline-source-{run_tag}",
        "unique_key": f"pipeline-tenant-{run_tag}:pipeline-source-{run_tag}:acme:{normalized}",
        "collected_at": "2026-01-05T00:00:00Z",
        "data_source": "insight_github",
        "org": "acme",
        "login": login,
        "login_normalized": normalized,
        "member_id": 7001,
        "name": display_name,
        "email": email,
        "company": "Example Corp",
        "role": "MEMBER",
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-04T00:00:00Z",
    }


def _run_connector_dbt_path(
    ch_seeder: CHSeeder,
    dbt_runner: DbtRunner,
    worker_ctx: WorkerContext,
    rows: list[dict],
) -> None:
    ch_seeder.truncate_touched()
    ch_seeder.seed_bronze(
        {"bronze_github_directory.org_members": rows}, _GITHUB_DIRECTORY_SCHEMAS
    )

    staging, silver = dbt_runner.derive_selectors({("bronze_github_directory", "org_members")})
    dbt_runner.build(" ".join(f"+{m}" for m in staging), worker_ctx=worker_ctx)
    assert "identity_inputs" in silver, (
        f"github_directory__identity_inputs did not surface a silver:identity_inputs tag "
        f"(silver={silver}) — the connector's identity path is not reachable from its bronze table"
    )
    dbt_runner.run("identity_inputs", worker_ctx=worker_ctx)


def _resolve_by_external_id(identity_svc, external_id: str) -> str | None:
    """Resolve exactly as the authenticator's login-bootstrap does."""
    with identity_svc.client(
        sub=str(seed.SEED_ADMIN), tenant=str(seed.SEED_TENANT), sub_type="service", roles="service"
    ) as svc:
        r = svc.get(
            "/internal/persons/by-external-id",
            params={"source_type": "github", "external_id": external_id},
        )
    if r.status_code == 404:
        return None
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    return r.json()["insight_source_id"]


def test_github_login_resolves_a_person_through_the_real_connector_pipeline(
    identity_svc,
    ch_seeder: CHSeeder,
    dbt_runner: DbtRunner,
    worker_ctx: WorkerContext,
    compose_stack: SessionConfig,
) -> None:
    """A seeded org member is resolvable by the id binding the authenticator
    queries — the whole reason this connector exists.
    """
    if not identity_svc.supports_seed_cli:
        pytest.skip("the seed CLI exists only on the Rust implementation (#1690)")

    run_tag = uuid.uuid4().hex[:10]
    login = f"pipeline-dev-{run_tag}"

    _run_connector_dbt_path(
        ch_seeder,
        dbt_runner,
        worker_ctx,
        [
            _org_member(
                run_tag=run_tag,
                login=login,
                email=f"pipeline.dev.{run_tag}@e2e.test",
                display_name="Pipeline Dev",
            )
        ],
    )

    landed = clickhouse.query(
        compose_stack,
        "SELECT count() FROM identity.identity_inputs"
        f" WHERE insight_source_type = 'github' AND source_account_id = '{login}'"
        "   AND value_type = 'id'",
    )
    assert landed[0][0] >= 1, (
        "the github-directory connector's dbt models never produced the canonical id row in "
        "identity.identity_inputs — the login binding does not exist, so every GitHub SSO "
        "callback would 403"
    )

    res = identity_svc.run_seed_cli(tenant=str(seed.SEED_TENANT), force=True)
    assert res.returncode == 0, f"rc={res.returncode}\n{res.stdout}\n{res.stderr}"

    assert _resolve_by_external_id(identity_svc, login) is not None, (
        "persons carries no row the login-bootstrap lookup can find for "
        f"(source_type=github, external_id={login})"
    )


def test_login_binding_is_lowercased_so_a_brokered_username_matches(
    identity_svc,
    ch_seeder: CHSeeder,
    dbt_runner: DbtRunner,
    worker_ctx: WorkerContext,
) -> None:
    """GitHub preserves the login's letter-case; Keycloak lowercases the
    username it brokers from it. `persons.value_id` is `COLLATE utf8mb4_bin`,
    so the comparison is byte-exact — storing GitHub's casing against a
    lowercased claim yields a 403 indistinguishable from missing data.

    The connector therefore binds on the lowercased login, and this pins it:
    the lowercased form resolves, the mixed-case one does not.
    """
    if not identity_svc.supports_seed_cli:
        pytest.skip("the seed CLI exists only on the Rust implementation (#1690)")

    run_tag = uuid.uuid4().hex[:10]
    mixed_case_login = f"Pipeline-Dev-{run_tag}"

    _run_connector_dbt_path(
        ch_seeder,
        dbt_runner,
        worker_ctx,
        [
            _org_member(
                run_tag=run_tag,
                login=mixed_case_login,
                email=f"pipeline.case.{run_tag}@e2e.test",
                display_name="Pipeline Case",
            )
        ],
    )

    res = identity_svc.run_seed_cli(tenant=str(seed.SEED_TENANT), force=True)
    assert res.returncode == 0, f"rc={res.returncode}\n{res.stdout}\n{res.stderr}"

    assert _resolve_by_external_id(identity_svc, mixed_case_login.lower()) is not None, (
        "a lowercased GitHub login must resolve — this is the form Keycloak brokers"
    )
    assert _resolve_by_external_id(identity_svc, mixed_case_login) is None, (
        "the mixed-case login resolved too, so the binding is not the normalized one this "
        "connector promises — re-check login_normalized and the fields_history entity_id"
    )
