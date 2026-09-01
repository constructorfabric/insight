"""The GitHub-brokered login path, end to end.

`tests/stand/api/identity/test_internal.py` already covers what
`/internal/persons/by-external-id` does with a row that exists: a service
principal resolves it, a person is refused. It says nothing about where that
row comes from — it reads a seeded manifest fixture, not a connector.

These tests cover the other half. They seed the GitHub member roster through
the REAL path (bronze -> github-directory connector dbt models -> the shared
`identity_inputs` union) and then resolve the way the authenticator does, so a
break anywhere in the chain that produces the binding surfaces here rather than
as a 403 at login. The second test pins WHICH value is the binding: the
member's immutable numeric GitHub id, never the login — a login resolving
would mean the entity key regressed to a renameable, reusable handle. The
third test pins the join with the `github` activity connector: a directory
member and a commit author carrying the same numeric id must land in ONE
identity account, because that meeting is the whole mechanism by which a
commit e-mail reaches a person.
"""

from __future__ import annotations

import uuid

import pytest
from lib import clickhouse
from lib import identity_seed as seed
from lib.ch_seeder import CHSeeder
from lib.config import SessionConfig
from lib.dbt_runner import DbtRunner
from lib.tracked_models import TrackedModels
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


# Declared from `scripts/connectors-ddl/github.sql` — no metrics fixture
# exists for this table either.
_GITHUB_ACTIVITY_SCHEMAS = {
    "bronze_github.commit_authors": {
        "properties": {
            "_airbyte_raw_id": {"type": "string"},
            "_airbyte_extracted_at": {"type": "string", "format": "date-time"},
            "_airbyte_meta": {"type": "string"},
            "_airbyte_generation_id": {"type": "integer"},
            "author_id": {"type": ["integer", "null"]},
            "source_id": {"type": ["string", "null"]},
            "tenant_id": {"type": ["string", "null"]},
            "sample_sha": {"type": ["string", "null"]},
            "unique_key": {"type": ["string", "null"]},
            "author_type": {"type": ["string", "null"]},
            "data_source": {"type": ["string", "null"]},
            "author_email": {"type": ["string", "null"]},
            "author_login": {"type": ["string", "null"]},
            "collected_at": {"type": ["string", "null"]},
            "repo_full_name": {"type": ["string", "null"]},
        }
    },
    "bronze_github.commits": {
        "properties": {
            "_airbyte_raw_id": {"type": "string"},
            "_airbyte_extracted_at": {"type": "string", "format": "date-time"},
            "_airbyte_meta": {"type": "string"},
            "_airbyte_generation_id": {"type": "integer"},
            "sha": {"type": ["string", "null"]},
            "message": {"type": ["string", "null"]},
            "author_name": {"type": ["string", "null"]},
            "author_email": {"type": ["string", "null"]},
            "authored_date": {"type": ["string", "null"]},
            "tenant_id": {"type": ["string", "null"]},
            "source_id": {"type": ["string", "null"]},
            "unique_key": {"type": ["string", "null"]},
            "repository": {"type": ["string", "null"]},
            "data_source": {"type": ["string", "null"]},
            "collected_at": {"type": ["string", "null"]},
        }
    },
}


def _run_specific_member_id(run_tag: str) -> int:
    """A member id unique to this run: `persons` is an append-only journal
    shared across runs, so a constant id would resolve against a previous
    run's binding and prove nothing."""
    return int(run_tag, 16)


def _org_member(*, run_tag: str, login: str, member_id: int, email: str, display_name: str) -> dict:
    """A minimal `bronze_github_directory.org_members` row, shaped the way the
    connector emits one: the entity key follows the immutable `member_id`,
    and the login rides along for display only.
    """
    return {
        "_airbyte_raw_id": str(uuid.uuid4()),
        "_airbyte_extracted_at": "2026-01-05T00:00:00",
        "_airbyte_meta": "{}",
        "_airbyte_generation_id": 0,
        "tenant_id": f"pipeline-tenant-{run_tag}",
        "source_id": f"pipeline-source-{run_tag}",
        "unique_key": f"pipeline-tenant-{run_tag}:pipeline-source-{run_tag}:acme:{member_id}",
        "collected_at": "2026-01-05T00:00:00Z",
        "data_source": "insight_github",
        "org": "acme",
        "login": login,
        "member_id": member_id,
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
    tracked_models: TrackedModels,
    worker_ctx: WorkerContext,
    bronze: dict[str, list[dict]],
) -> None:
    schemas = {**_GITHUB_DIRECTORY_SCHEMAS, **_GITHUB_ACTIVITY_SCHEMAS}
    ch_seeder.truncate_touched()
    ch_seeder.seed_bronze(bronze, {fqn: schemas[fqn] for fqn in bronze})

    seeded = {tuple(fqn.split(".", 1)) for fqn in bronze}
    staging, silver = dbt_runner.derive_selectors(seeded)
    tracked_models.build(staging, worker_ctx=worker_ctx, with_ancestors=True)
    assert "identity_inputs" in silver, (
        f"github_directory__identity_inputs did not surface a silver:identity_inputs tag "
        f"(silver={silver}) — the connector's identity path is not reachable from its bronze table"
    )
    tracked_models.run(["identity_inputs"], worker_ctx=worker_ctx)


def _person_id_by_email(identity_svc, email: str) -> str | None:
    """The person an address names, the second way in.

    Used to say WHICH person a member id must resolve to: the id and the
    roster address are two independent handles on one member, so agreeing on
    a person is a claim `is not None` cannot make.
    """
    with identity_svc.client(
        sub=str(seed.SEED_ADMIN), tenant=str(seed.SEED_TENANT), sub_type="service", roles="service"
    ) as svc:
        r = svc.get("/internal/persons/by-email-override", params={"email": email})
    if r.status_code == 404:
        return None
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    return r.json()["insight_source_id"]


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


def test_github_member_id_resolves_a_person_through_the_real_connector_pipeline(
    identity_svc,
    ch_seeder: CHSeeder,
    dbt_runner: DbtRunner,
    tracked_models: TrackedModels,
    worker_ctx: WorkerContext,
    compose_stack: SessionConfig,
) -> None:
    """A seeded org member is resolvable by the id binding the authenticator
    queries — the whole reason this connector exists.
    """
    run_tag = uuid.uuid4().hex[:10]
    member_id = _run_specific_member_id(run_tag)
    login = f"pipeline-dev-{run_tag}"

    _run_connector_dbt_path(
        ch_seeder,
        dbt_runner,
        tracked_models,
        worker_ctx,
        {
            "bronze_github_directory.org_members": [
                _org_member(
                    run_tag=run_tag,
                    login=login,
                    member_id=member_id,
                    email=f"pipeline.dev.{run_tag}@e2e.test",
                    display_name="Pipeline Dev",
                )
            ]
        },
    )

    landed = clickhouse.query(
        compose_stack,
        "SELECT count() FROM identity.identity_inputs"
        f" WHERE insight_source_type = 'github' AND source_account_id = '{member_id}'"
        "   AND value_type = 'id'",
    )
    assert landed[0][0] >= 1, (
        "the github-directory connector's dbt models never produced the canonical id row in "
        "identity.identity_inputs — the member-id binding does not exist, so every GitHub SSO "
        "callback would 403"
    )

    username_rows = clickhouse.query(
        compose_stack,
        "SELECT value FROM identity.identity_inputs"
        f" WHERE insight_source_type = 'github' AND source_account_id = '{member_id}'"
        "   AND value_type = 'username' AND operation_type = 'UPSERT'",
    )
    assert [r[0] for r in username_rows] == [login], (
        f"expected exactly one username observation carrying the raw login {login!r} under "
        f"account {member_id} (got {username_rows!r}) — the handle is what the identity "
        "console displays and searches now that the binding is numeric"
    )

    res = identity_svc.run_seed_cli(tenant=str(seed.SEED_TENANT), force=True)
    assert res.returncode == 0, f"rc={res.returncode}\n{res.stdout}\n{res.stderr}"

    resolved = _resolve_by_external_id(identity_svc, str(member_id))
    assert resolved is not None, (
        "persons carries no row the login-bootstrap lookup can find for "
        f"(source_type=github, external_id={member_id})"
    )

    # WHICH person, not merely some person. The member id and the roster
    # address are two independent handles on the same member, so a sign-in
    # landing on a real-but-wrong person — the failure this criterion exists
    # to rule out — satisfies `is not None` and fails this.
    by_address = _person_id_by_email(identity_svc, f"pipeline.dev.{run_tag}@e2e.test")
    assert by_address is not None, "the roster address names no person"
    assert resolved == by_address, (
        f"the member id resolves to {resolved}, the roster address to {by_address} — "
        "a sign-in through this connector lands on someone other than the member"
    )


def test_the_binding_is_the_member_id_never_the_login(
    identity_svc,
    ch_seeder: CHSeeder,
    dbt_runner: DbtRunner,
    tracked_models: TrackedModels,
    worker_ctx: WorkerContext,
) -> None:
    """GitHub logins are renameable by their owner and re-registrable once
    freed; the numeric member id is neither. The connector therefore binds on
    the id, and this pins it: the id resolves, the login (in any casing) does
    not — a login resolving means the fields_history entity key regressed to
    the handle and sign-in is one rename away from breaking.
    """
    run_tag = uuid.uuid4().hex[:10]
    member_id = _run_specific_member_id(run_tag)
    mixed_case_login = f"Pipeline-Dev-{run_tag}"

    _run_connector_dbt_path(
        ch_seeder,
        dbt_runner,
        tracked_models,
        worker_ctx,
        {
            "bronze_github_directory.org_members": [
                _org_member(
                    run_tag=run_tag,
                    login=mixed_case_login,
                    member_id=member_id,
                    email=f"pipeline.case.{run_tag}@e2e.test",
                    display_name="Pipeline Case",
                )
            ]
        },
    )

    res = identity_svc.run_seed_cli(tenant=str(seed.SEED_TENANT), force=True)
    assert res.returncode == 0, f"rc={res.returncode}\n{res.stdout}\n{res.stderr}"

    assert _resolve_by_external_id(identity_svc, str(member_id)) is not None, (
        "the numeric member id must resolve — this is the claim the broker's "
        "`jsonField: id` mapper carries"
    )
    for login_form in (mixed_case_login, mixed_case_login.lower()):
        assert _resolve_by_external_id(identity_svc, login_form) is None, (
            f"the login {login_form!r} resolved, so the binding is not the member id this "
            "connector promises — re-check the fields_history entity_id"
        )


def test_directory_and_activity_claims_meet_in_one_account(
    ch_seeder: CHSeeder,
    dbt_runner: DbtRunner,
    tracked_models: TrackedModels,
    worker_ctx: WorkerContext,
    compose_stack: SessionConfig,
) -> None:
    """A directory member and a commit author carrying the same numeric id
    must converge on ONE identity account: the roster contributes the binding
    and the username, the activity connector contributes the e-mail claim.

    This is the deployment contract behind sharing one `insight_source_id`
    (`github-main`) across both connectors — and the mechanism by which a
    member whose profile hides their e-mail still gets commit attribution.
    The member here publishes no e-mail, so the address below can reach a
    person ONLY through this join.

    The pre-2017 noreply address rides along: it names only the login, and
    must resolve to the same numeric account through the login_to_id map
    (here fed by the roster's login+member_id pair).
    """
    run_tag = uuid.uuid4().hex[:10]
    member_id = _run_specific_member_id(run_tag)
    login = f"Pipeline-Meet-{run_tag}"
    commit_email = f"pipeline.meet.{run_tag}@e2e.test"
    legacy_noreply = f"{login.lower()}@users.noreply.github.com"

    _run_connector_dbt_path(
        ch_seeder,
        dbt_runner,
        tracked_models,
        worker_ctx,
        {
            "bronze_github_directory.org_members": [
                _org_member(
                    run_tag=run_tag,
                    login=login,
                    member_id=member_id,
                    email="",
                    display_name="Pipeline Meet",
                )
            ],
            "bronze_github.commit_authors": [
                {
                    "_airbyte_raw_id": str(uuid.uuid4()),
                    "_airbyte_extracted_at": "2026-01-05T00:00:00",
                    "_airbyte_meta": "{}",
                    "_airbyte_generation_id": 0,
                    "author_id": member_id,
                    "source_id": f"pipeline-source-{run_tag}",
                    "tenant_id": f"pipeline-tenant-{run_tag}",
                    "sample_sha": "0" * 40,
                    "unique_key": f"pipeline-tenant-{run_tag}:pipeline-source-{run_tag}:{member_id}:{commit_email}",
                    "author_type": "User",
                    "data_source": "insight_github",
                    "author_email": commit_email,
                    "author_login": login,
                    "collected_at": "2026-01-05T00:00:00Z",
                    "repo_full_name": "acme/repo",
                }
            ],
            "bronze_github.commits": [
                {
                    "_airbyte_raw_id": str(uuid.uuid4()),
                    "_airbyte_extracted_at": "2026-01-05T00:00:00",
                    "_airbyte_meta": "{}",
                    "_airbyte_generation_id": 0,
                    "sha": "1" * 40,
                    "message": "legacy noreply commit",
                    "author_name": "Pipeline Meet",
                    "author_email": legacy_noreply,
                    "authored_date": "2026-01-05T00:00:00Z",
                    "tenant_id": f"pipeline-tenant-{run_tag}",
                    "source_id": f"pipeline-source-{run_tag}",
                    "unique_key": f"pipeline-tenant-{run_tag}:pipeline-source-{run_tag}:{'1' * 40}",
                    "repository": "acme/repo",
                    "data_source": "insight_github",
                    "collected_at": "2026-01-05T00:00:00Z",
                }
            ],
        },
    )

    observed = clickhouse.query(
        compose_stack,
        "SELECT value_type, value FROM identity.identity_inputs"
        f" WHERE insight_source_type = 'github' AND source_account_id = '{member_id}'"
        "   AND operation_type = 'UPSERT'",
    )
    pairs = {(value_type, value) for value_type, value in observed}

    expected = {
        ("id", str(member_id)),
        ("username", login),
        ("email", commit_email),
        ("email", legacy_noreply),
    }
    assert expected <= pairs, (
        f"account {member_id} carries {sorted(pairs)!r}, expected it to carry at least "
        f"{sorted(expected)!r} — the roster binding and the activity connector's e-mail claim "
        "did not meet in one account, so a hidden-e-mail member's commits reach no person"
    )
