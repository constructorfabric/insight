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

The last test covers the one shape that collides with itself: a user who
publishes the SAME address as both `email` and `public_email`. The claim key
does not distinguish the two fields, so without a dedup inside the run they are
two rows under one `unique_key` — which the anti join against the target cannot
catch, because neither row is in the target yet.
"""

from __future__ import annotations

import uuid
from dataclasses import dataclass
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

#: A bronze cell, as ClickHouse takes it from the seeder.
type BronzeValue = str | int | bool | None
#: One bronze row: column name to value.
type BronzeRow = dict[str, BronzeValue]
#: A JSON-schema document, as the metrics rig declares one per bronze table.
type TableSchemas = dict[str, dict[str, object]]
#: The seeder's input: rows to insert, keyed by fully-qualified table name.
type BronzeTables = dict[str, list[BronzeRow]]

# The metrics rig already declares every column of these three tables.
_SCHEMA_DIR = Path(__file__).resolve().parents[1] / "metrics" / "schemas"

READ_AT = "2026-10-02T00:00:00Z"


@dataclass(frozen=True)
class Envelope:
    """The CDK and routing columns every bronze row this test seeds carries.

    Its own tenant and connection per run: the shared union keeps every run's
    claims, and `source_account_id` is only unique within one connection.
    """

    run_tag: str
    unique_key: str

    def row(self) -> BronzeRow:
        """Render the envelope as the bronze columns the seeder writes."""
        return {
            "_airbyte_raw_id": str(uuid.uuid4()),
            "_airbyte_extracted_at": READ_AT,
            "_airbyte_meta": "{}",
            "_airbyte_generation_id": 0,
            "tenant_id": f"gitlab-tenant-{self.run_tag}",
            "source_id": f"gitlab-source-{self.run_tag}",
            "data_source": "insight_gitlab",
            "collected_at": READ_AT,
            "unique_key": self.unique_key,
        }


def _schemas(tables: list[str]) -> TableSchemas:
    """Load the metrics rig's column declarations for the given bronze tables."""
    merged: TableSchemas = {}
    for table in tables:
        doc = yaml.safe_load((_SCHEMA_DIR / f"{table}.yaml").read_text(encoding="utf-8"))
        merged |= doc["schemas"]
    return merged


def _user(run_tag: str, user_id: int, name: str, email: str = "", public_email: str = "") -> BronzeRow:
    """A GitLab user record, publishing whichever addresses it was given."""
    return Envelope(run_tag, f"u-{user_id}").row() | {
        "id": user_id,
        "username": f"user-{user_id}",
        "name": name,
        "state": "active",
        "email": email,
        "public_email": public_email,
        "bot": False,
    }


def _request(run_tag: str, iid: int, author_id: int) -> BronzeRow:
    """A merged merge request opened by the given account."""
    return Envelope(run_tag, f"mr-{iid}").row() | {
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


def _commit(run_tag: str, sha: str, iid: int, author_email: str) -> BronzeRow:
    """A commit the given merge request carries, authored under `author_email`."""
    return Envelope(run_tag, f"mc-{sha}").row() | {
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
    """A per-run account id, so a claim from another run cannot answer for this one."""
    return 700_000 + int(run_tag[:5], 16) % 100_000


def _run_connector_dbt_path(
    ch_seeder: CHSeeder,
    dbt_runner: DbtRunner,
    tracked_models: TrackedModels,
    worker_ctx: WorkerContext,
    bronze: BronzeTables,
) -> None:
    """Seed bronze, build the connector's models, then the shared union.

    `build` runs the models' data tests too, so a duplicate `unique_key` fails
    here rather than surviving into an assertion.
    """
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


def _claims(cfg: SessionConfig, account_id: int) -> list[tuple[str, str, str]]:
    """Every claim the shared union holds for one GitLab account."""
    rows = clickhouse.query(
        cfg,
        "SELECT value_type, value, value_field_name FROM identity.identity_inputs"
        f" WHERE insight_source_type = 'gitlab' AND source_account_id = '{account_id}'"
        "   AND operation_type = 'UPSERT' ORDER BY value_type, value",
    )
    return [(row[0], row[1], row[2]) for row in rows]


def test_gitlab_publishes_its_account_claims_and_infers_none_from_commits(
    ch_seeder: CHSeeder,
    dbt_runner: DbtRunner,
    tracked_models: TrackedModels,
    worker_ctx: WorkerContext,
    compose_stack: SessionConfig,
) -> None:
    """The account's own address and name arrive; a commit's address does not."""
    run_tag = uuid.uuid4().hex[:10]
    account_id = _account_id(run_tag)
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
            USERS: [_user(run_tag, account_id, display_name, email=published)],
            REQUESTS: [_request(run_tag, 1, account_id)],
            COMMITS: [_commit(run_tag, f"sha-{run_tag}", 1, commit_author)],
        },
    )

    assert [(kind, value) for kind, value, _ in _claims(compose_stack, account_id)] == [
        ("display_name", display_name),
        ("email", published),
    ], "the account's own e-mail and name are what the persons-seed binds on"

    inferred = clickhouse.query(
        compose_stack,
        "SELECT count() FROM identity.identity_inputs"
        f" WHERE insight_source_type = 'gitlab' AND value = '{commit_author}'",
    )
    assert int(inferred[0][0]) == 0, (
        "an address from the request's commits was claimed for an account — a commit says "
        "nothing about who opened the request, and the claim cannot be withdrawn later"
    )


def test_one_address_published_in_both_fields_is_one_claim(
    ch_seeder: CHSeeder,
    dbt_runner: DbtRunner,
    tracked_models: TrackedModels,
    worker_ctx: WorkerContext,
    compose_stack: SessionConfig,
) -> None:
    """A user showing the same address twice yields one claim, not a key collision."""
    run_tag = uuid.uuid4().hex[:10]
    account_id = _account_id(run_tag)
    published = f"both.{run_tag}@e2e.test"

    # Reaching the assertions at all is half the proof: the model's `unique`
    # data test runs inside this build, so two rows under one key fail here.
    _run_connector_dbt_path(
        ch_seeder,
        dbt_runner,
        tracked_models,
        worker_ctx,
        {USERS: [_user(run_tag, account_id, "Both Fields", email=published, public_email=published)]},
    )

    emails = [(value, field) for kind, value, field in _claims(compose_stack, account_id) if kind == "email"]
    assert emails == [(published, "bronze_gitlab.users.email")], (
        "one address published in both fields must be one claim, and its provenance must not "
        "depend on the order the rows happened to arrive in"
    )
