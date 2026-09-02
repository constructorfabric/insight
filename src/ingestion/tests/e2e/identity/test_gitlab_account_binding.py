"""What GitLab says about a user account reaches the shared identity inputs.

`gitlab__pull_requests` keys a merge request's author on the numeric GitLab
user id, and the account-first resolution in gold reads that key. The key is
worth nothing until something binds it to a person, and the persons-seed binds
from `identity.identity_inputs` — so this test drives the real path (bronze ->
the connector's dbt models -> the shared `identity_inputs` union) and asserts
the claims arrive there under that same numeric id.

It also pins what must NOT arrive. A merge request's commits are evidence about
whoever wrote them, never about whoever opened the request, and the claims here
are append-only: an address inferred from a request's commits could hand one
person's address to another and could not be withdrawn afterwards. So the
fixture gives the request a commit written under a second address and expects
no account to claim it.
"""

from __future__ import annotations

import uuid
from pathlib import Path

import pytest
import yaml
from lib import clickhouse
from lib.ch_seeder import CHSeeder
from lib.config import SessionConfig
from lib.dbt_runner import DbtRunner
from lib.tracked_models import TrackedModels
from lib.worker import WorkerContext

pytestmark = [pytest.mark.identity, pytest.mark.mutating]

USERS = "bronze_gitlab.users"
REQUESTS = "bronze_gitlab.merge_requests"
COMMITS = "bronze_gitlab.merge_request_commits"

# The metrics rig already declares every column of these three tables.
_SCHEMA_DIR = Path(__file__).resolve().parents[1] / "metrics" / "schemas"

READ_AT = "2026-10-02T00:00:00Z"


def _schemas(tables: list[str]) -> dict:
    merged: dict = {}
    for table in tables:
        doc = yaml.safe_load((_SCHEMA_DIR / f"{table}.yaml").read_text(encoding="utf-8"))
        merged |= doc["schemas"]
    return merged


def _envelope(run_tag: str, unique_key: str) -> dict:
    return {
        "_airbyte_raw_id": str(uuid.uuid4()),
        "_airbyte_extracted_at": READ_AT,
        "_airbyte_meta": "{}",
        "_airbyte_generation_id": 0,
        # Its own tenant and connection per run: the shared union keeps every
        # run's claims, and `source_account_id` is only unique within one.
        "tenant_id": f"gitlab-tenant-{run_tag}",
        "source_id": f"gitlab-source-{run_tag}",
        "data_source": "insight_gitlab",
        "collected_at": READ_AT,
        "unique_key": unique_key,
    }


def _run_connector_dbt_path(
    ch_seeder: CHSeeder,
    dbt_runner: DbtRunner,
    tracked_models: TrackedModels,
    worker_ctx: WorkerContext,
    bronze: dict[str, list[dict]],
) -> None:
    ch_seeder.truncate_touched()
    ch_seeder.seed_bronze(bronze, _schemas(list(bronze)))

    seeded = {tuple(fqn.split(".", 1)) for fqn in bronze}
    staging, silver = dbt_runner.derive_selectors(seeded)
    tracked_models.build(staging, worker_ctx=worker_ctx, with_ancestors=True)
    assert "identity_inputs" in silver, (
        f"gitlab__identity_inputs did not surface a silver:identity_inputs tag (silver={silver}) "
        "— the connector's identity path is not reachable from its bronze tables"
    )
    tracked_models.run(["identity_inputs"], worker_ctx=worker_ctx)


def test_gitlab_publishes_its_account_claims_and_infers_none_from_commits(
    ch_seeder: CHSeeder,
    dbt_runner: DbtRunner,
    tracked_models: TrackedModels,
    worker_ctx: WorkerContext,
    compose_stack: SessionConfig,
) -> None:
    run_tag = uuid.uuid4().hex[:10]
    # A per-run id, so a claim from another run cannot answer for this one.
    account_id = 700_000 + int(run_tag[:5], 16) % 100_000
    published = f"author.{run_tag}@e2e.test"
    display_name = "Request Author"
    # The address on the request's commit, written by somebody else.
    commit_author = f"committer.{run_tag}@e2e.test"

    _run_connector_dbt_path(
        ch_seeder,
        dbt_runner,
        tracked_models,
        worker_ctx,
        {
            USERS: [
                _envelope(run_tag, f"u-{account_id}")
                | {
                    "id": account_id,
                    "username": f"author-{run_tag}",
                    "name": display_name,
                    "state": "active",
                    "email": published,
                    "public_email": "",
                    "bot": False,
                }
            ],
            REQUESTS: [
                _envelope(run_tag, f"mr-{account_id}")
                | {
                    "project_id": 101,
                    "iid": 1,
                    "id": 90_001,
                    "title": "change",
                    "state": "merged",
                    "author_id": account_id,
                    "author_username": f"author-{run_tag}",
                    "source_branch": "feature",
                    "target_branch": "main",
                    "created_at": "2026-10-01T08:00:00Z",
                    "updated_at": "2026-10-01T13:00:00Z",
                    "merged_at": "2026-10-01T09:00:00Z",
                }
            ],
            COMMITS: [
                _envelope(run_tag, f"mc-{account_id}")
                | {
                    "project_id": 101,
                    "mr_iid": 1,
                    "id": f"sha-{run_tag}",
                    "short_id": run_tag[:8],
                    "title": "change",
                    "message": "change",
                    "author_name": "Someone Else",
                    "author_email": commit_author,
                    "authored_date": "2026-10-01T08:30:00Z",
                    "committer_name": "Someone Else",
                    "committer_email": commit_author,
                    "committed_date": "2026-10-01T08:30:00Z",
                }
            ],
        },
    )

    claims = clickhouse.query(
        compose_stack,
        "SELECT value_type, value FROM identity.identity_inputs"
        f" WHERE insight_source_type = 'gitlab' AND source_account_id = '{account_id}'"
        "   AND operation_type = 'UPSERT' ORDER BY value_type, value",
    )
    assert [tuple(row) for row in claims] == [("display_name", display_name), ("email", published)], (
        "the account's own e-mail and name are what the persons-seed binds on"
    )

    inferred = clickhouse.query(
        compose_stack,
        "SELECT count() FROM identity.identity_inputs"
        f" WHERE insight_source_type = 'gitlab' AND value = '{commit_author}'",
    )
    assert int(inferred[0][0]) == 0, (
        "an address from the request's commits was claimed for an account — a commit says "
        "nothing about who opened the request, and the claim cannot be withdrawn later"
    )
