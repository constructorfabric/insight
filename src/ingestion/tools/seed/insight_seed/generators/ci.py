"""GitHub Actions runs and deployments, into bronze.

Runs are anchored to the commits and pull requests `git_history` produced, so
the CI-to-commit join the lens charts is a real match rate. Push, schedule and
manual runs carry a seeded commit's SHA; pull-request and merge-queue runs
carry a synthetic merge ref, which is what the vendor builds and the commits
stream never observes.

`CI_WINDOW_DAYS` clamps runs and deployments to the vendor's ~90-day workflow
retention, independent of the seed's own `days` window: a freshly seeded
stand is a first sync, so its CI history cannot honestly reach further back
than the source could deliver.
"""

from __future__ import annotations

import datetime as _dt
import math
import random
from collections.abc import Sequence
from typing import TYPE_CHECKING

from ..profiles import Person
from .base import (
    UTC,
    anchor_date,
    anchor_datetime,
    deterministic_int,
    deterministic_uuid,
    seeded_rng,
)
from .ci_topology import Pipeline, Repo, ci_repos, repo_grid
from .git_history import DayHistory, PullRequest
from .git_repos import GITHUB_SOURCE_ID
from .insert import bulk_insert, truncate

if TYPE_CHECKING:
    import clickhouse_connect.driver.client

CI_WINDOW_DAYS = 90

_OUTCOMES: tuple[tuple[str | None, float], ...] = (
    ("success", 0.69),
    ("failure", 0.15),
    ("timed_out", 0.02),
    ("cancelled", 0.05),
    ("skipped", 0.03),
    ("action_required", 0.02),
    (None, 0.04),
)

_DURATION_SIGMA = 1.8
_ZERO_DURATION_SHARE = 0.03
_RETRY_SHARE = 0.07
_MERGE_QUEUE_SHARE = 0.2

_DEPLOY_STATES: tuple[tuple[str, float], ...] = (
    ("success", 0.80),
    ("failure", 0.14),
    ("error", 0.06),
)


def _pick[T](rng: random.Random, weighted: Sequence[tuple[T, float]]) -> T:
    roll = rng.random() * sum(w for _, w in weighted)
    for value, weight in weighted:
        roll -= weight
        if roll <= 0:
            return value
    return weighted[-1][0]


def _duration_s(rng: random.Random, pipeline: Pipeline, conclusion: str | None) -> int:
    if conclusion == "action_required" or rng.random() < _ZERO_DURATION_SHARE:
        return 0
    return max(1, int(math.exp(math.log(pipeline.median_s) + rng.gauss(0, _DURATION_SIGMA))))


def _in_window(day: _dt.date) -> bool:
    return day > anchor_date() - _dt.timedelta(days=CI_WINDOW_DAYS)


_RUN_COLS = [
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
    "run_attempt",
    "repo_full_name",
    "name",
    "workflow_id",
    "event",
    "status",
    "conclusion",
    "head_branch",
    "head_sha",
    "actor_login",
    "run_number",
    "workflow_path",
    "display_title",
    "pull_request_numbers",
    "run_started_at",
    "created_at",
    "updated_at",
]


def _run_row(
    tenant_uuid: str,
    repo: Repo,
    pipeline: Pipeline,
    run_id: int,
    attempt: int,
    event: str,
    conclusion: str | None,
    head_sha: str,
    branch: str,
    actor: str,
    started: _dt.datetime,
    duration_s: int,
) -> tuple[object, ...]:
    finished = started + _dt.timedelta(seconds=duration_s)
    fields: dict[str, object] = {
        "_airbyte_raw_id": deterministic_uuid("ci.run.raw", str(run_id), str(attempt)),
        "_airbyte_extracted_at": anchor_datetime(),
        "_airbyte_meta": "{}",
        "_airbyte_generation_id": 0,
        "unique_key": f"{tenant_uuid}:{GITHUB_SOURCE_ID}:{repo.full_name}:run:{run_id}:{attempt}",
        "tenant_id": tenant_uuid,
        "source_id": GITHUB_SOURCE_ID,
        "data_source": "insight_github",
        "collected_at": anchor_datetime().isoformat(),
        "id": run_id,
        "run_attempt": attempt,
        "repo_full_name": repo.full_name,
        "name": pipeline.name,
        "workflow_id": deterministic_int("ci.workflow", repo.full_name, pipeline.path),
        "event": event,
        "status": "completed" if conclusion is not None else "in_progress",
        "conclusion": conclusion,
        "head_branch": branch,
        "head_sha": head_sha,
        "actor_login": actor,
        "run_number": run_id % 10_000,
        "workflow_path": pipeline.path,
        "display_title": f"{pipeline.name} on {branch}",
        "pull_request_numbers": "[]",
        "run_started_at": started.isoformat(),
        "created_at": started.isoformat(),
        "updated_at": finished.isoformat(),
    }
    return tuple(fields.get(col) for col in _RUN_COLS)


def _attempts(
    rng: random.Random,
    tenant_uuid: str,
    repo: Repo,
    pipeline: Pipeline,
    run_id: int,
    event: str,
    head_sha: str,
    branch: str,
    actor: str,
    started: _dt.datetime,
) -> list[tuple[object, ...]]:
    """One bronze row per attempt.

    Half of the retried runs also carry their earlier, failed attempt: the
    source lists only the latest attempt, but a sync that observed the
    earlier one before the retry landed leaves both rows in bronze.
    """
    conclusion = _pick(rng, _OUTCOMES)
    # INVARIANT: conclusion is drawn before the retry roll — reordering
    # shifts every later draw this function and its caller make.
    retried = rng.random() < _RETRY_SHARE
    attempt = 2 if retried else 1
    rows = [
        _run_row(
            tenant_uuid,
            repo,
            pipeline,
            run_id,
            attempt,
            event,
            conclusion,
            head_sha,
            branch,
            actor,
            started,
            _duration_s(rng, pipeline, conclusion),
        )
    ]
    if retried and rng.random() < 0.5:
        rows.append(
            _run_row(
                tenant_uuid,
                repo,
                pipeline,
                run_id,
                1,
                event,
                "failure",
                head_sha,
                branch,
                actor,
                started - _dt.timedelta(minutes=20),
                _duration_s(rng, pipeline, "failure"),
            )
        )
    return rows


def seed_workflow_runs(
    client: clickhouse_connect.driver.client.Client,
    history: Sequence[DayHistory],
    grid: Sequence[Repo],
    tenant_uuid: str,
) -> int:
    truncate(client, "bronze_github", "workflow_runs")
    repos = ci_repos(grid)
    weighted = [(r, r.weight) for r in repos]
    rows: list[tuple[object, ...]] = []

    for day in history:
        if not _in_window(day.date):
            continue
        actor = day.person.email.split("@", 1)[0]
        for index, pr in enumerate(day.prs):
            rng = seeded_rng(day.person.uuid, day.date, f"ci.pr.{index}")
            repo = _pick(rng, weighted)
            branch = f"feature/pr-{pr.pr_id}"
            merge_ref = deterministic_uuid("ci.mergeref", str(pr.pr_id))[:40].replace("-", "")

            for pipeline in (p for p in repo.pipelines if "pull_request" in p.triggers):
                for update in range(1 + rng.randint(0, 2)):
                    run_id = deterministic_int("ci.run", str(pr.pr_id), pipeline.path, str(update))
                    started = pr.created + _dt.timedelta(hours=update * 3)
                    rows.extend(
                        _attempts(
                            rng,
                            tenant_uuid,
                            repo,
                            pipeline,
                            run_id,
                            "pull_request",
                            merge_ref,
                            branch,
                            actor,
                            started,
                        )
                    )

            if pr.merged_on is None or not day.commits:
                continue
            landed = day.commits[index % len(day.commits)].hash
            gate_event = (
                "merge_group"
                if repo.full_name == "acme/platform" and rng.random() < _MERGE_QUEUE_SHARE
                else "push"
            )
            for pipeline in (p for p in repo.pipelines if "push" in p.triggers):
                run_id = deterministic_int("ci.run.push", str(pr.pr_id), pipeline.path)
                rows.extend(
                    _attempts(
                        rng,
                        tenant_uuid,
                        repo,
                        pipeline,
                        run_id,
                        gate_event,
                        merge_ref if gate_event == "merge_group" else landed,
                        repo.default_branch,
                        actor,
                        pr.merged_on,
                    )
                )

    rows.extend(_scheduled_and_manual_runs(history, repos, tenant_uuid))
    return bulk_insert(client, "bronze_github", "workflow_runs", _RUN_COLS, rows)


def _scheduled_and_manual_runs(
    history: Sequence[DayHistory], repos: Sequence[Repo], tenant_uuid: str
) -> list[tuple[object, ...]]:
    """Nightly and manual runs, walked by repository rather than by person.

    The nightly cluster exists on every in-window day regardless of who
    committed; manual dispatch is rare, at roughly one day in ten.
    """
    commits_by_day: dict[_dt.date, str] = {}
    for entry in history:
        if entry.commits:
            commits_by_day[entry.date] = entry.commits[-1].hash

    rows: list[tuple[object, ...]] = []
    for repo in repos:
        for day, head_sha in sorted(commits_by_day.items()):
            if not _in_window(day):
                continue
            rng = seeded_rng(repo.full_name, day, "ci.cron")
            for pipeline in repo.pipelines:
                if "schedule" in pipeline.triggers:
                    run_id = deterministic_int(
                        "ci.run.cron", repo.full_name, pipeline.path, day.isoformat()
                    )
                    started = _dt.datetime.combine(day, _dt.time(2, 0, tzinfo=UTC))
                    rows.extend(
                        _attempts(
                            rng,
                            tenant_uuid,
                            repo,
                            pipeline,
                            run_id,
                            "schedule",
                            head_sha,
                            repo.default_branch,
                            "github-actions",
                            started,
                        )
                    )
                if "workflow_dispatch" in pipeline.triggers and rng.random() < 0.1:
                    run_id = deterministic_int(
                        "ci.run.manual", repo.full_name, pipeline.path, day.isoformat()
                    )
                    started = _dt.datetime.combine(day, _dt.time(16, 0, tzinfo=UTC))
                    rows.extend(
                        _attempts(
                            rng,
                            tenant_uuid,
                            repo,
                            pipeline,
                            run_id,
                            "workflow_dispatch",
                            head_sha,
                            repo.default_branch,
                            "release-bot",
                            started,
                        )
                    )
    return rows


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
        if not _in_window(day.date):
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
    and a tenth reach production. All but the newest production deployment
    carry a status event: the newest is left pending, and a production
    deployment superseded by another within a day reports `inactive` rather
    than its rolled outcome.
    """
    truncate(client, "bronze_github", "deployments")
    truncate(client, "bronze_github", "deployment_statuses")
    repos = ci_repos(grid)
    weighted = [(r, r.weight) for r in repos]
    merged = _merged_prs(history)

    deployments: list[tuple[object, ...]] = []
    statuses: list[tuple[object, ...]] = []
    production_ids: list[tuple[_dt.datetime, int, str, Repo]] = []

    for position, (day, pr, index, merged_on) in enumerate(merged):
        rng = seeded_rng(day.person.uuid, day.date, f"ci.deploy.{index}")
        repo = _pick(rng, weighted)
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
                        _pick(rng, _DEPLOY_STATES),
                        created + _dt.timedelta(minutes=4),
                        day,
                    )
                )

    production_ids.sort()
    for position, (created, deploy_id, env, repo) in enumerate(production_ids[:-1]):
        following = production_ids[position + 1][0]
        superseded = following - created < _dt.timedelta(days=1)
        rng = seeded_rng(repo.full_name, created.date(), f"ci.depstatus.{deploy_id}")
        state = "inactive" if superseded else _pick(rng, _DEPLOY_STATES)
        statuses.append(
            _status_row(
                tenant_uuid, repo, deploy_id, env, state, created + _dt.timedelta(minutes=4), None
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


def generate(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
    history: Sequence[DayHistory],
) -> dict[str, int]:
    grid = repo_grid(roster)
    deployments, statuses = seed_deployments(client, history, grid, tenant_uuid)
    return {
        "bronze_github.workflow_runs": seed_workflow_runs(client, history, grid, tenant_uuid),
        "bronze_github.deployments": deployments,
        "bronze_github.deployment_statuses": statuses,
    }
