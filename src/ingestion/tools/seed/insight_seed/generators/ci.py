"""GitHub Actions workflow runs, into bronze.

Runs are anchored to the commits and pull requests `git_history` produced, so
the CI-to-commit join the lens charts is a real match rate. Push, schedule and
manual runs carry a seeded commit's SHA; pull-request and merge-queue runs
carry a synthetic merge ref, which is what the vendor builds and the commits
stream never observes.
"""

from __future__ import annotations

import datetime as _dt
import math
import random
from collections.abc import Sequence
from typing import TYPE_CHECKING

from ..profiles import Person
from .base import UTC, anchor_datetime, deterministic_int, deterministic_uuid, pick, seeded_rng
from .ci_deployments import seed_deployments

# WORKAROUND: self-aliased so mypy/ruff treat it as an explicit re-export —
# ci.CI_WINDOW_DAYS is read directly by tests/test_ci_shapes.py. `nopycln`
# because pycln does not recognise the self-alias convention on its own.
from .ci_topology import CI_WINDOW_DAYS as CI_WINDOW_DAYS  # nopycln: import
from .ci_topology import Pipeline, Repo, ci_repos, in_window, repo_grid
from .git_history import DayHistory
from .git_repos import GITHUB_SOURCE_ID
from .insert import bulk_insert, truncate

if TYPE_CHECKING:
    import clickhouse_connect.driver.client

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


def _duration_s(rng: random.Random, pipeline: Pipeline, conclusion: str | None) -> int:
    if conclusion == "action_required" or rng.random() < _ZERO_DURATION_SHARE:
        return 0
    return max(1, int(math.exp(math.log(pipeline.median_s) + rng.gauss(0, _DURATION_SIGMA))))


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
    conclusion = pick(rng, _OUTCOMES)
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
    truncate(client, "staging", "github__ci_runs")
    # INVARIANT: this model declines a dbt full refresh because in production
    # it archives history past the source API's retention. A stand is not an
    # archive: without this, a re-seed at a different anchor strands the
    # previous window's runs permanently.
    truncate(client, "silver", "class_git_ci_runs")
    repos = ci_repos(grid)
    weighted = [(r, r.weight) for r in repos]
    rows: list[tuple[object, ...]] = []

    for day in history:
        if not in_window(day.date):
            continue
        actor = day.person.email.split("@", 1)[0]
        for index, pr in enumerate(day.prs):
            rng = seeded_rng(day.person.uuid, day.date, f"ci.pr.{index}")
            repo = pick(rng, weighted)
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
            if not in_window(day):
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
