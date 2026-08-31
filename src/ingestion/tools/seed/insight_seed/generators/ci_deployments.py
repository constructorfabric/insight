"""GitHub deployments and their status events, into bronze.

Deployments index off the merged pull requests `git_history` produced, so the
mix mirrors the commit history's own pace rather than a separate schedule.
"""

from __future__ import annotations

import datetime as _dt
from collections.abc import Sequence
from typing import TYPE_CHECKING

from .base import anchor_datetime, deterministic_int, deterministic_uuid, pick, seeded_rng
from .ci_topology import Repo, ci_repos, in_window
from .git_history import DayHistory, PullRequest
from .git_repos import GITHUB_SOURCE_ID
from .insert import bulk_insert, truncate

if TYPE_CHECKING:
    import clickhouse_connect.driver.client

_DEPLOY_STATES: tuple[tuple[str, float], ...] = (
    ("success", 0.80),
    ("failure", 0.14),
    ("error", 0.06),
)

_DEPLOY_COLS = [
    "_airbyte_raw_id",
    "_airbyte_extracted_at",
    "_airbyte_meta",
    "_airbyte_generation_id",
    "unique_key",
    "tenant_id",
    "source_id",
    "data_source",
    "collected_at",
    "id",
    "repo_full_name",
    "sha",
    "ref",
    "task",
    "environment",
    "original_environment",
    "is_transient_environment",
    "is_production_environment",
    "creator_login",
    "created_at",
    "updated_at",
]

_STATUS_COLS = [
    "_airbyte_raw_id",
    "_airbyte_extracted_at",
    "_airbyte_meta",
    "_airbyte_generation_id",
    "unique_key",
    "tenant_id",
    "source_id",
    "data_source",
    "collected_at",
    "id",
    "deployment_id",
    "repo_full_name",
    "state",
    "environment",
    "creator_login",
    "created_at",
    "updated_at",
]


def _merged_prs(
    history: Sequence[DayHistory],
) -> list[tuple[DayHistory, PullRequest, int, _dt.datetime]]:
    """Merged PRs in the CI window, ordered by merge time then PR id.

    Carries the already-narrowed `merged_on` alongside each entry so the sort
    key and every downstream read are typed `datetime`, not `datetime | None`.
    """
    merged: list[tuple[DayHistory, PullRequest, int, _dt.datetime]] = []
    for day in history:
        if not in_window(day.date):
            continue
        for index, pr in enumerate(day.prs):
            if pr.merged_on is None or not day.commits:
                continue
            merged.append((day, pr, index, pr.merged_on))
    merged.sort(key=lambda item: (item[3], item[1].pr_id))
    return merged


def seed_deployments(
    client: clickhouse_connect.driver.client.Client,
    history: Sequence[DayHistory],
    grid: Sequence[Repo],
    tenant_uuid: str,
) -> tuple[int, int]:
    """Deployments and their status events, indexed off merged pull requests.

    Every merge deploys to a preview environment; a third also reach staging
    and a tenth reach production. Within each repository's production
    environment, all but the newest deployment carry a status event: the
    newest is left pending, and a deployment superseded by another to the
    same repository's production environment within a day reports `inactive`
    rather than its rolled outcome.
    """
    truncate(client, "bronze_github", "deployments")
    truncate(client, "bronze_github", "deployment_statuses")
    truncate(client, "staging", "github__deployments")
    truncate(client, "staging", "github__deployment_events")
    # INVARIANT: these models decline a dbt full refresh because in production
    # they archive history past the source API's retention. A stand is not an
    # archive: without this, a re-seed at a different anchor strands the
    # previous window's runs permanently.
    truncate(client, "silver", "class_git_deployments")
    truncate(client, "silver", "class_git_deployment_events")
    repos = ci_repos(grid)
    weighted = [(r, r.weight) for r in repos]
    merged = _merged_prs(history)

    deployments: list[tuple[object, ...]] = []
    statuses: list[tuple[object, ...]] = []
    production_ids: list[tuple[_dt.datetime, int, str, Repo]] = []

    for position, (day, pr, index, merged_on) in enumerate(merged):
        rng = seeded_rng(day.person.uuid, day.date, f"ci.deploy.{index}")
        repo = pick(rng, weighted)
        sha = day.commits[index % len(day.commits)].hash
        envs: list[tuple[str, bool, bool]] = [(f"preview-{pr.pr_id % 50}", True, False)]
        if position % 3 == 0:
            envs.append(("staging", False, False))
        if position % 10 == 0:
            envs.append(("production", False, True))

        for env, transient, production in envs:
            deploy_id = deterministic_int("ci.deploy", repo.full_name, env, str(pr.pr_id))
            created = merged_on + _dt.timedelta(minutes=12)
            deployments.append(
                tuple(
                    {
                        "_airbyte_raw_id": deterministic_uuid("ci.deploy.raw", str(deploy_id)),
                        "_airbyte_extracted_at": anchor_datetime(),
                        "_airbyte_meta": "{}",
                        "_airbyte_generation_id": 0,
                        "unique_key": (
                            f"{tenant_uuid}:{GITHUB_SOURCE_ID}:{repo.full_name}:deploy:{deploy_id}"
                        ),
                        "tenant_id": tenant_uuid,
                        "source_id": GITHUB_SOURCE_ID,
                        "data_source": "insight_github",
                        "collected_at": anchor_datetime().isoformat(),
                        "id": deploy_id,
                        "repo_full_name": repo.full_name,
                        "sha": sha,
                        "ref": repo.default_branch,
                        "task": "deploy",
                        "environment": env,
                        "original_environment": env,
                        "is_transient_environment": transient,
                        "is_production_environment": production,
                        "creator_login": day.person.email.split("@", 1)[0],
                        "created_at": created.isoformat(),
                        "updated_at": created.isoformat(),
                    }.get(col)
                    for col in _DEPLOY_COLS
                )
            )
            if production:
                production_ids.append((created, deploy_id, env, repo))
            else:
                statuses.append(
                    _status_row(
                        tenant_uuid,
                        repo,
                        deploy_id,
                        env,
                        pick(rng, _DEPLOY_STATES),
                        created + _dt.timedelta(minutes=4),
                        day,
                    )
                )

    by_repo_environment: dict[tuple[str, str], list[tuple[_dt.datetime, int, str, Repo]]] = {}
    for entry in production_ids:
        _created, _deploy_id, entry_env, entry_repo = entry
        by_repo_environment.setdefault((entry_repo.full_name, entry_env), []).append(entry)

    for group in by_repo_environment.values():
        group.sort(key=lambda item: (item[0], item[1]))
        for position, (created, deploy_id, env, repo) in enumerate(group[:-1]):
            following = group[position + 1][0]
            superseded = following - created < _dt.timedelta(days=1)
            rng = seeded_rng(repo.full_name, created.date(), f"ci.depstatus.{deploy_id}")
            state = "inactive" if superseded else pick(rng, _DEPLOY_STATES)
            statuses.append(
                _status_row(
                    tenant_uuid,
                    repo,
                    deploy_id,
                    env,
                    state,
                    created + _dt.timedelta(minutes=4),
                    None,
                )
            )

    return (
        bulk_insert(client, "bronze_github", "deployments", _DEPLOY_COLS, deployments),
        bulk_insert(client, "bronze_github", "deployment_statuses", _STATUS_COLS, statuses),
    )


def _status_row(
    tenant_uuid: str,
    repo: Repo,
    deployment_id: int,
    environment: str,
    state: str,
    at: _dt.datetime,
    day: DayHistory | None,
) -> tuple[object, ...]:
    event_id = deterministic_int("ci.depstatus", str(deployment_id), state, at.isoformat())
    creator = day.person.email.split("@", 1)[0] if day else "deploy-bot"
    fields: dict[str, object] = {
        "_airbyte_raw_id": deterministic_uuid("ci.depstatus.raw", str(event_id)),
        "_airbyte_extracted_at": anchor_datetime(),
        "_airbyte_meta": "{}",
        "_airbyte_generation_id": 0,
        "unique_key": (
            f"{tenant_uuid}:{GITHUB_SOURCE_ID}:{repo.full_name}:{deployment_id}:status:{event_id}"
        ),
        "tenant_id": tenant_uuid,
        "source_id": GITHUB_SOURCE_ID,
        "data_source": "insight_github",
        "collected_at": anchor_datetime().isoformat(),
        "id": event_id,
        "deployment_id": deployment_id,
        "repo_full_name": repo.full_name,
        "state": state,
        "environment": environment,
        "creator_login": creator,
        "created_at": at.isoformat(),
        "updated_at": at.isoformat(),
    }
    return tuple(fields.get(col) for col in _STATUS_COLS)
