"""Which repositories exist, which run CI, and what pipelines they run.

`CI_WINDOW_DAYS`/`in_window` clamp runs and deployments to the vendor's
~90-day workflow retention, independent of the seed's own `days` window: a
freshly seeded stand is a first sync, so its CI history cannot honestly reach
further back than the source could deliver.
"""

from __future__ import annotations

import datetime as _dt
from collections.abc import Sequence
from dataclasses import dataclass, field

from ..profiles import Person
from .base import anchor_date

# INVARIANT: Event names must match GitHub exactly; merge_group maps to merge_queue trigger_category in connectors/git/github/dbt/github__ci_runs.sql.
COMMIT_TRIGGERS = ("push", "pull_request", "merge_group")

CI_WINDOW_DAYS = 90


def in_window(day: _dt.date) -> bool:
    """Whether `day` falls inside the vendor's CI retention window."""
    return day > anchor_date() - _dt.timedelta(days=CI_WINDOW_DAYS)


@dataclass(frozen=True)
class Pipeline:
    path: str
    name: str
    median_s: int
    triggers: tuple[str, ...]


@dataclass(frozen=True)
class Repo:
    full_name: str
    vendor: str
    default_branch: str
    weight: float = 0.0
    pipelines: tuple[Pipeline, ...] = field(default_factory=tuple)


_BUILD = Pipeline(".github/workflows/ci.yml", "Build & test", 480, COMMIT_TRIGGERS)
_E2E = Pipeline(".github/workflows/e2e.yml", "End-to-end", 1500, COMMIT_TRIGGERS)
_RELEASE = Pipeline(".github/workflows/release.yml", "Release", 300, ("workflow_dispatch",))
_NIGHTLY = Pipeline(".github/workflows/nightly.yml", "Nightly", 2100, ("schedule",))
_LINT = Pipeline(".github/workflows/lint.yml", "Lint", 60, COMMIT_TRIGGERS)
_LINKS = Pipeline(".github/workflows/link-check.yml", "Link check", 90, ("schedule",))

BASE_REPOS: tuple[Repo, ...] = (
    Repo("acme/platform", "github", "main", 0.55, (_BUILD, _E2E, _RELEASE, _NIGHTLY)),
    Repo("acme/gateway", "github", "main", 0.25, (_BUILD, _RELEASE)),
    Repo("acme/docs", "github", "main", 0.12, (_LINT, _LINKS)),
    Repo("acme/tooling", "github", "main", 0.08, (_BUILD,)),
    Repo("acme/legacy-archive", "github", "main"),
    Repo("acme-analytics/data-pipeline", "gitlab", "main"),
    Repo("acme-mobile/client-sdk", "bitbucket", "develop"),
)


def _growth_squads(roster: Sequence[Person]) -> int:
    """Development squads past the founding one.

    `scale.py` puts every growth squad under its own manager carrying the
    `lead` role, so counting them is how repository count follows headcount.
    """
    leads = [p for p in roster if p.team == "development" and p.role == "lead"]
    return max(0, len(leads) - 1)


def repo_grid(roster: Sequence[Person]) -> tuple[Repo, ...]:
    """The base grid plus one GitHub repository per growth squad."""
    grown = [
        Repo(f"acme/squad-{n}", "github", "main", 0.0, (_BUILD,))
        for n in range(1, _growth_squads(roster) + 1)
    ]
    if not grown:
        return BASE_REPOS

    share = 0.2 / len(grown)
    rescaled = tuple(
        Repo(r.full_name, r.vendor, r.default_branch, r.weight * 0.8, r.pipelines)
        if r.weight
        else r
        for r in BASE_REPOS
    )
    return rescaled + tuple(
        Repo(r.full_name, r.vendor, r.default_branch, share, r.pipelines) for r in grown
    )


def ci_repos(grid: Sequence[Repo]) -> tuple[Repo, ...]:
    """Repositories that actually run pipelines."""
    return tuple(r for r in grid if r.pipelines)
