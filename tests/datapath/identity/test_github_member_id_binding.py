"""The GitHub member roster reaches identity through the connector's own models, and the
binding it leaves is the member's immutable numeric id: the login lookup resolves the id
and never the login, and a directory member and a commit author with one id are one
account, which is how a commit e-mail reaches a person.
"""

from __future__ import annotations

from typing import Any

import pytest
from insight_datapath import clickhouse, records
from insight_datapath.connector_path import ConnectorPath
from insight_datapath.instance import InstanceConfig
from insight_datapath.subjects import Subjects
from insight_stand.api import ApiClient

pytestmark = pytest.mark.fixture

ORG_MEMBERS = "bronze_github_directory.org_members"
COMMIT_AUTHORS = "bronze_github.commit_authors"
COMMITS = "bronze_github.commits"

SOURCE_TYPE = "github"
BY_EXTERNAL_ID = "/internal/persons/by-external-id"

REPO = "acme/repo"

PROFILE_CLAIMS = ("person_display_name", "person_email", "person_username")


def _member_id(run_tag: str) -> int:
    """Run-unique: the journal is append-only, so a constant id would resolve a past run."""
    return int(run_tag, 16)


def _source_id(run_tag: str) -> str:
    return f"github-directory-{run_tag}"


def _commit_author(
    run_tag: str, *, login: str, member_id: int, email: str, tenant: str
) -> dict[str, Any]:
    source_id = _source_id(run_tag)
    return {
        **records.framing(),
        "tenant_id": tenant,
        "source_id": source_id,
        "unique_key": f"{tenant}:{source_id}:{member_id}:{email}",
        "data_source": "insight_github",
        "collected_at": records.OBSERVED_AT,
        "repo_full_name": REPO,
        "author_email": email,
        "author_login": login,
        "author_id": member_id,
        "author_type": "User",
        "sample_sha": "0" * 40,
    }


def _commit(run_tag: str, *, author_name: str, author_email: str, tenant: str) -> dict[str, Any]:
    source_id = _source_id(run_tag)
    sha = "1" * 40
    return {
        **records.framing(),
        "tenant_id": tenant,
        "source_id": source_id,
        "unique_key": f"{tenant}:{source_id}:{sha}",
        "repository": REPO,
        "sha": sha,
        "author_name": author_name,
        "author_email": author_email,
        "committer_name": author_name,
        "committer_email": author_email,
        "message": "legacy noreply commit",
        "authored_date": records.OBSERVED_AT,
        "committed_date": records.OBSERVED_AT,
        "is_merge": False,
    }


def _account_claims(cfg: InstanceConfig, member_id: int) -> list[tuple[str, str]]:
    """Every (value_type, value) the identity inputs assert about the member's account."""
    rows = clickhouse.query(
        cfg,
        "SELECT value_type, value FROM identity.identity_inputs"
        f" WHERE insight_source_type = '{SOURCE_TYPE}' AND source_account_id = '{member_id}'"
        "   AND operation_type = 'UPSERT'"
        " ORDER BY value_type, value",
    )
    return [(str(value_type), str(value)) for value_type, value in rows]


def _resolved_person(service_client: ApiClient, external_id: str) -> str | None:
    """The person the login lookup names for `external_id`, or None when it names nobody."""
    response = service_client.get(
        BY_EXTERNAL_ID, params={"source_type": SOURCE_TYPE, "external_id": external_id}
    )
    if response.status_code == 404:
        return None
    assert response.status_code == 200, (
        f"external_id={external_id!r}: {response.status_code} {response.text[:300]}"
    )

    body = response.json()
    assert isinstance(body, dict), f"external_id={external_id!r}: {response.text[:300]}"
    return str(body["insight_source_id"])


@pytest.mark.requires_service_principal
def test_a_member_id_resolves_a_person_through_the_connector_pipeline(
    connector_path: ConnectorPath,
    instance_cfg: InstanceConfig,
    subjects: Subjects,
    service_client: ApiClient,
    tenant: str,
    run_tag: str,
) -> None:
    """A rostered member's numeric id is the binding the login lookup resolves, and it
    names the same person the member's roster address does."""
    member_id = _member_id(run_tag)
    login = f"pipeline-dev-{run_tag}"
    email = f"pipeline.dev.{run_tag}@example.com"

    connector_path.build(
        {
            ORG_MEMBERS: [
                records.org_member(
                    source_id=_source_id(run_tag),
                    login=login,
                    member_id=member_id,
                    email=email,
                    name="Pipeline Dev",
                    tenant=tenant,
                )
            ]
        }
    )

    claims = _account_claims(instance_cfg, member_id)
    assert [value for value_type, value in claims if value_type == "id"] == [str(member_id)], (
        f"account {member_id} carries {claims!r}, not the one canonical id row the login "
        "lookup matches"
    )
    assert [value for value_type, value in claims if value_type == "username"] == [login], (
        f"account {member_id} carries {claims!r}, not exactly one username row holding {login!r}"
    )

    subjects.publish()

    resolved = _resolved_person(service_client, str(member_id))
    assert resolved is not None, (
        f"(source_type={SOURCE_TYPE}, external_id={member_id}) names nobody"
    )
    by_address = subjects.person_ids([email]).get(email)
    assert by_address is not None, f"the roster address {email} names nobody"
    assert resolved == by_address, (
        f"the member id resolves to {resolved}, the roster address to {by_address}: a sign-in "
        "through this connector lands on someone other than the member"
    )


def test_the_directory_emits_roster_profile_claims(
    connector_path: ConnectorPath, instance_cfg: InstanceConfig, tenant: str, run_tag: str
) -> None:
    """A member's name, address and login each become one canonical profile claim."""
    member_id = _member_id(run_tag)
    login = f"profile-dev-{run_tag}"
    email = f"profile.dev.{run_tag}@example.com"
    name = "Profile Dev"

    connector_path.build(
        {
            ORG_MEMBERS: [
                records.org_member(
                    source_id=_source_id(run_tag),
                    login=login,
                    member_id=member_id,
                    email=email,
                    name=name,
                    tenant=tenant,
                )
            ]
        }
    )

    claims = _account_claims(instance_cfg, member_id)
    profile = [claim for claim in claims if claim[0] in PROFILE_CLAIMS]
    assert profile == [
        ("person_display_name", name),
        ("person_email", email),
        ("person_username", login),
    ], f"account {member_id} carries the profile claims {profile!r}"


@pytest.mark.requires_service_principal
def test_the_binding_is_the_member_id_never_the_login(
    connector_path: ConnectorPath,
    subjects: Subjects,
    service_client: ApiClient,
    tenant: str,
    run_tag: str,
) -> None:
    """A login is renameable and reusable, so only the immutable numeric id resolves;
    the login resolves in no casing."""
    member_id = _member_id(run_tag)
    mixed_case_login = f"Pipeline-Dev-{run_tag}"

    connector_path.build(
        {
            ORG_MEMBERS: [
                records.org_member(
                    source_id=_source_id(run_tag),
                    login=mixed_case_login,
                    member_id=member_id,
                    email=f"pipeline.case.{run_tag}@example.com",
                    name="Pipeline Case",
                    tenant=tenant,
                )
            ]
        }
    )
    subjects.publish()

    assert _resolved_person(service_client, str(member_id)) is not None, (
        f"the numeric member id {member_id} names nobody"
    )
    for login_form in (mixed_case_login, mixed_case_login.lower()):
        assert _resolved_person(service_client, login_form) is None, (
            f"the login {login_form!r} resolved, so the binding is not the member id"
        )


def test_directory_and_activity_claims_meet_in_one_account(
    connector_path: ConnectorPath, instance_cfg: InstanceConfig, tenant: str, run_tag: str
) -> None:
    """A directory member and a commit author carrying one numeric id are one account:
    the roster's binding and login meet the activity connector's e-mail claims there,
    the pre-2017 noreply address resolved through the login included."""
    member_id = _member_id(run_tag)
    login = f"Pipeline-Meet-{run_tag}"
    commit_email = f"pipeline.meet.{run_tag}@example.com"
    legacy_noreply = f"{login.lower()}@users.noreply.github.com"

    connector_path.build(
        {
            ORG_MEMBERS: [
                records.org_member(
                    source_id=_source_id(run_tag),
                    login=login,
                    member_id=member_id,
                    email="",
                    name="Pipeline Meet",
                    tenant=tenant,
                )
            ],
            COMMIT_AUTHORS: [
                _commit_author(
                    run_tag, login=login, member_id=member_id, email=commit_email, tenant=tenant
                )
            ],
            COMMITS: [
                _commit(
                    run_tag,
                    author_name="Pipeline Meet",
                    author_email=legacy_noreply,
                    tenant=tenant,
                )
            ],
        }
    )

    observed = set(_account_claims(instance_cfg, member_id))
    expected = {
        ("id", str(member_id)),
        ("username", login),
        ("email", commit_email),
        ("email", legacy_noreply),
    }
    assert expected <= observed, (
        f"account {member_id} carries {sorted(observed)!r}, expected at least "
        f"{sorted(expected)!r}: the roster binding and the activity connector's e-mail claims "
        "did not meet in one account, so a hidden-e-mail member's commits reach no person"
    )
