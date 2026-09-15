"""A Bitbucket account that acts on a pull request reaches identity as a bindable
account: the connector states its account id, the seed attaches it to the person
its verified commit address names, and the account lookup then resolves the
pull request author to that person. An account seen only through a commit e-mail
states no id and stays a claim.
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

EMPLOYEES = "bronze_bamboohr.employees"
COMMIT_AUTHORS = "bronze_bitbucket_cloud.commit_authors"
PULL_REQUESTS = "bronze_bitbucket_cloud.pull_requests"

SOURCE_TYPE = "bitbucket"
BY_EXTERNAL_ID = "/internal/persons/by-external-id"

REPO = "acme/app"


def _source_id(run_tag: str) -> str:
    return f"bitbucket-cloud-{run_tag}"


def _account(run_tag: str, who: str) -> str:
    """Run-unique: the journal is append-only, so a constant id would resolve a past run."""
    return f"{who}-{run_tag}"


def _commit_author(run_tag: str, *, account: str, email: str, tenant: str) -> dict[str, Any]:
    source_id = _source_id(run_tag)
    return {
        **records.framing(),
        "tenant_id": tenant,
        "source_id": source_id,
        "unique_key": f"{tenant}:{source_id}:{REPO}:{email}",
        "data_source": "insight_bitbucket_cloud",
        "collected_at": records.OBSERVED_AT,
        "repo_full_name": REPO,
        "author_email": email,
        "author_account_id": account,
        "author_uuid": "{" + account + "}",
        "author_nickname": account,
        "author_display_name": account,
        "sample_sha": "0" * 40,
        "last_committed_date": records.OBSERVED_AT,
    }


def _pull_request(run_tag: str, *, pr_id: int, account: str, tenant: str) -> dict[str, Any]:
    source_id = _source_id(run_tag)
    return {
        **records.framing(),
        "tenant_id": tenant,
        "source_id": source_id,
        "unique_key": f"{tenant}:{source_id}:{REPO}:{pr_id}",
        "data_source": "insight_bitbucket_cloud",
        "collected_at": records.OBSERVED_AT,
        "id": pr_id,
        "repo_full_name": REPO,
        "title": f"PR {pr_id}",
        "state": "MERGED",
        "draft": False,
        "author_account_id": account,
        "author_uuid": "{" + account + "}",
        "author_display_name": account,
        "created_on": "2026-01-02T10:00:00+00:00",
        "updated_on": "2026-01-03T10:00:00+00:00",
        "source_branch": "feat",
        "destination_branch": "main",
        "merge_commit_sha": "a" * 12,
        "source_commit_sha": "b" * 12,
        "destination_commit_sha": "c" * 12,
        "comment_count": 0,
        "task_count": 0,
    }


def _account_claims(cfg: InstanceConfig, account: str) -> list[tuple[str, str]]:
    """Every (value_type, value) the identity inputs assert about the account."""
    rows = clickhouse.query(
        cfg,
        "SELECT value_type, value FROM identity.identity_inputs"
        f" WHERE insight_source_type = '{SOURCE_TYPE}' AND source_account_id = '{account}'"
        "   AND operation_type = 'UPSERT'"
        " ORDER BY value_type, value",
    )
    return [(str(value_type), str(value)) for value_type, value in rows]


def _resolved_person(service_client: ApiClient, external_id: str) -> str | None:
    """The person the account lookup names for `external_id`, or None when it names nobody."""
    response = service_client.get(BY_EXTERNAL_ID, params={"source_type": SOURCE_TYPE, "external_id": external_id})
    if response.status_code == 404:
        return None
    assert response.status_code == 200, f"external_id={external_id!r}: {response.status_code} {response.text[:300]}"

    body = response.json()
    assert isinstance(body, dict), f"external_id={external_id!r}: {response.text[:300]}"
    return str(body["insight_source_id"])


def test_a_pull_request_author_states_an_id_and_a_commit_only_account_does_not(
    connector_path: ConnectorPath, instance_cfg: InstanceConfig, tenant: str, run_tag: str
) -> None:
    """The binding row exists for the account that acted on a pull request and for no
    account the connector knows only through a commit address."""
    author = _account(run_tag, "author")
    passerby = _account(run_tag, "passerby")

    connector_path.build(
        {
            COMMIT_AUTHORS: [
                _commit_author(run_tag, account=author, email=f"author.{run_tag}@example.com", tenant=tenant),
                _commit_author(run_tag, account=passerby, email=f"passerby.{run_tag}@example.org", tenant=tenant),
            ],
            PULL_REQUESTS: [_pull_request(run_tag, pr_id=1, account=author, tenant=tenant)],
        }
    )

    author_claims = _account_claims(instance_cfg, author)
    assert [value for value_type, value in author_claims if value_type == "id"] == [author], (
        f"account {author} carries {author_claims!r}, not the one binding row the lookup matches"
    )
    assert f"author.{run_tag}@example.com" in {v for k, v in author_claims if k == "email"}, (
        f"account {author} must keep its e-mail claim beside the binding: {author_claims!r}"
    )

    passerby_claims = _account_claims(instance_cfg, passerby)
    assert [value for value_type, value in passerby_claims if value_type == "id"] == [], (
        f"a commit-only account is a claim, not a participant: {passerby_claims!r}"
    )
    assert [value for value_type, value in passerby_claims if value_type == "email"] == [
        f"passerby.{run_tag}@example.org"
    ], passerby_claims


@pytest.mark.requires_service_principal
def test_a_pull_request_author_resolves_to_the_person_their_commit_address_names(
    connector_path: ConnectorPath,
    subjects: Subjects,
    substitutions: dict[str, str],
    service_client: ApiClient,
    tenant: str,
    run_tag: str,
) -> None:
    """Bitbucket names no address on a pull request. The account id it names binds
    to the person the account's verified commit address already belongs to, so
    the request reaches that person through the account lookup."""
    author = _account(run_tag, "author")
    email = f"pipeline.dev.{run_tag}@example.com"

    connector_path.build(
        {
            EMPLOYEES: [
                records.employee(
                    substitutions=substitutions, key=f"bb-{run_tag}", email=email, display_name="Pipeline Dev"
                )
            ],
            COMMIT_AUTHORS: [_commit_author(run_tag, account=author, email=email, tenant=tenant)],
            PULL_REQUESTS: [_pull_request(run_tag, pr_id=2, account=author, tenant=tenant)],
        }
    )

    subjects.publish()

    resolved = _resolved_person(service_client, author)
    assert resolved is not None, f"(source_type={SOURCE_TYPE}, external_id={author}) names nobody"
    by_address = subjects.person_ids([email]).get(email)
    assert by_address is not None, f"the roster address {email} names nobody"
    assert resolved == by_address, (
        f"the account resolves to {resolved}, the roster address to {by_address}: the pull "
        "request lands on someone other than its author"
    )
