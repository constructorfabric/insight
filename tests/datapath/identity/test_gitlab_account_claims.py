"""What GitLab says about a user account reaches the shared identity inputs.

The claims a user record publishes arrive in `identity.identity_inputs` under the
numeric user id that `gitlab__pull_requests` keys a merge request's author on; a
merge request's commits contribute no claim, since a commit says nothing about who
opened the request and a claim cannot be withdrawn; and one address published as
both `email` and `public_email` is one claim, attributed to `email`.
"""

from __future__ import annotations

import uuid
from dataclasses import dataclass

import pytest
from insight_datapath import clickhouse
from insight_datapath.connector_path import ConnectorPath
from insight_datapath.instance import InstanceConfig

pytestmark = pytest.mark.fixture

USERS = "bronze_gitlab.users"
REQUESTS = "bronze_gitlab.merge_requests"
COMMITS = "bronze_gitlab.merge_request_commits"

type BronzeValue = str | int | bool | None
type BronzeRow = dict[str, BronzeValue]

READ_AT = "2026-10-02T00:00:00Z"


@dataclass(frozen=True)
class Envelope:
    tenant: str
    run_tag: str

    def row(self, unique_key: str) -> BronzeRow:
        return {
            "_airbyte_raw_id": str(uuid.uuid4()),
            "_airbyte_extracted_at": READ_AT,
            "_airbyte_meta": "{}",
            "_airbyte_generation_id": 0,
            "tenant_id": self.tenant,
            "source_id": f"gitlab-{self.run_tag}",
            "data_source": "insight_gitlab",
            "collected_at": READ_AT,
            "unique_key": unique_key,
        }


def _user(
    envelope: Envelope, user_id: int, name: str, email: str = "", public_email: str = ""
) -> BronzeRow:
    return envelope.row(f"u-{user_id}") | {
        "id": user_id,
        "username": f"user-{user_id}",
        "name": name,
        "state": "active",
        "email": email,
        "public_email": public_email,
        "bot": False,
    }


def _request(envelope: Envelope, iid: int, author_id: int) -> BronzeRow:
    return envelope.row(f"mr-{iid}") | {
        "project_id": 101,
        "iid": iid,
        "id": 90_000 + iid,
        "title": "change",
        "state": "merged",
        "author_id": author_id,
        "author_username": f"user-{author_id}",
        "source_branch": "feature",
        "target_branch": "main",
        "created_at": "2026-10-01T08:00:00Z",
        "updated_at": "2026-10-01T13:00:00Z",
        "merged_at": "2026-10-01T09:00:00Z",
    }


def _commit(envelope: Envelope, sha: str, iid: int, author_email: str) -> BronzeRow:
    return envelope.row(f"mc-{sha}") | {
        "project_id": 101,
        "mr_iid": iid,
        "id": sha,
        "short_id": sha[:8],
        "title": "change",
        "message": "change",
        "author_name": "Someone Else",
        "author_email": author_email,
        "authored_date": "2026-10-01T08:30:00Z",
        "committer_name": "Someone Else",
        "committer_email": author_email,
        "committed_date": "2026-10-01T08:30:00Z",
    }


def _account_id(run_tag: str) -> int:
    return 700_000 + int(run_tag[:5], 16) % 100_000


def _claims(cfg: InstanceConfig, account_id: int) -> list[tuple[str, str, str]]:
    rows = clickhouse.query(
        cfg,
        "SELECT value_type, value, value_field_name FROM identity.identity_inputs"
        f" WHERE insight_source_type = 'gitlab' AND source_account_id = '{account_id}'"
        "   AND operation_type = 'UPSERT' ORDER BY value_type, value",
    )
    return [(row[0], row[1], row[2]) for row in rows]


def test_gitlab_publishes_its_account_claims_and_infers_none_from_commits(
    connector_path: ConnectorPath, instance_cfg: InstanceConfig, tenant: str, run_tag: str
) -> None:
    """The account's own address and name arrive; the address on a commit its merge
    request carries does not."""
    envelope = Envelope(tenant, run_tag)
    account_id = _account_id(run_tag)
    published = f"author.{run_tag}@example.com"
    display_name = "Request Author"
    commit_author = f"committer.{run_tag}@example.com"

    connector_path.build(
        {
            USERS: [_user(envelope, account_id, display_name, email=published)],
            REQUESTS: [_request(envelope, 1, account_id)],
            COMMITS: [_commit(envelope, f"sha-{run_tag}", 1, commit_author)],
        }
    )

    claims = [(kind, value) for kind, value, _ in _claims(instance_cfg, account_id)]
    assert claims == [("display_name", display_name), ("email", published)], (
        "the account's own e-mail and name are what the persons-seed binds on"
    )

    inferred = clickhouse.query(
        instance_cfg,
        "SELECT count() FROM identity.identity_inputs"
        f" WHERE insight_source_type = 'gitlab' AND value = '{commit_author}'",
    )
    assert int(inferred[0][0]) == 0, (
        "an address from the request's commits was claimed for an account — a commit says "
        "nothing about who opened the request, and the claim cannot be withdrawn later"
    )


def test_one_address_published_in_both_fields_is_one_claim(
    connector_path: ConnectorPath, instance_cfg: InstanceConfig, tenant: str, run_tag: str
) -> None:
    """A user showing the same address twice yields one claim attributed to `email`; the
    build runs the model's `unique` data test, so two rows under one key fail there."""
    envelope = Envelope(tenant, run_tag)
    account_id = _account_id(run_tag)
    published = f"both.{run_tag}@example.com"

    connector_path.build(
        {
            USERS: [
                _user(envelope, account_id, "Both Fields", email=published, public_email=published)
            ]
        }
    )

    claims = _claims(instance_cfg, account_id)
    emails = [(value, field) for kind, value, field in claims if kind == "email"]
    assert emails == [(published, "bronze_gitlab.users.email")], (
        "one address published in both fields must be one claim, and its provenance must not "
        "depend on the order the rows happened to arrive in"
    )
