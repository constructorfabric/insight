"""
git silver-table generator: commits + pull-requests.

Only the development team produces git activity. Sales / HR / Support
get zero rows here by construction.
"""

from __future__ import annotations

import datetime as _dt
from collections.abc import Sequence
from typing import TYPE_CHECKING

from generators.base import (
    bulk_insert,
    clamp,
    days_window,
    deterministic_int,
    deterministic_uuid,
    persona_multiplier,
    poisson,
    seeded_rng,
    truncate,
    weekday_multiplier,
)
from profiles import TEAM_PROFILES, Person

if TYPE_CHECKING:
    import clickhouse_connect.driver.client


# Hard per-person-per-day caps. Generation respects these by
# construction — they aren't validation rules, just upper bounds on
# the Poisson draws so the dataset stays plausible.
COMMITS_CAP = 20
PRS_CAP = 6


def _eligible(roster: Sequence[Person]) -> list[Person]:
    """Persons whose team profile has any git weight."""
    return [
        p for p in roster
        if p.team and TEAM_PROFILES[p.team].weights.get("github", 0) > 0
    ]


def seed_class_git_commits(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> int:
    truncate(client, "silver", "class_git_commits")
    cols = [
        "insight_tenant_id", "commit_hash", "project_key", "repo_slug",
        "tenant_id", "author_email", "date", "is_merge_commit",
        "file_path", "lines_added", "lines_removed", "data_source", "_version",
    ]
    rows: list[tuple[object, ...]] = []
    version = 1
    for p in _eligible(roster):
        persona = persona_multiplier(p.uuid)
        weight = TEAM_PROFILES[p.team or ""].weights["github"]
        for d in days_window(days):
            rng = seeded_rng(p.uuid, d, "git.commits")
            mean = 5 * persona * weight * weekday_multiplier(d)
            n_commits = min(poisson(rng, mean), COMMITS_CAP)
            for i in range(n_commits):
                sha = deterministic_uuid("git.commit", p.uuid, d.isoformat(), str(i))[:40]
                is_merge = 1 if rng.random() < 0.05 else 0
                # LOC per commit capped at ≤200 by construction.
                added = float(rng.randint(2, 180))
                removed = float(rng.randint(0, 80))
                rows.append((
                    tenant_uuid, sha.replace("-", ""), "insight", "insight/insight",
                    tenant_uuid, p.email, d, is_merge,
                    "src/main.rs", added, removed, "insight_github", version,
                ))
    return bulk_insert(client, "silver", "class_git_commits", cols, rows)


def seed_class_git_pull_requests(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> int:
    truncate(client, "silver", "class_git_pull_requests")
    cols = [
        "insight_tenant_id", "pr_id", "author_email", "author_name",
        "state", "created_on", "merged_on", "closed_on",
        "lines_added", "lines_removed", "tenant_id", "data_source", "_version",
    ]
    rows: list[tuple[object, ...]] = []
    version = 1
    for p in _eligible(roster):
        persona = persona_multiplier(p.uuid)
        weight = TEAM_PROFILES[p.team or ""].weights["github"]
        author_name = p.email.split("@", 1)[0].replace("_", " ").title()
        for d in days_window(days):
            rng = seeded_rng(p.uuid, d, "git.prs")
            mean = 0.8 * persona * weight * weekday_multiplier(d)
            n_prs = min(poisson(rng, mean), PRS_CAP)
            for i in range(n_prs):
                pr_id = deterministic_int("git.pr", p.uuid, d.isoformat(), str(i))
                created = _dt.datetime.combine(
                    d, _dt.time(9 + rng.randint(0, 8), rng.randint(0, 59), tzinfo=_dt.UTC),
                )
                merged_in_h = rng.randint(1, 72) if rng.random() < 0.85 else None
                merged_on = (
                    created + _dt.timedelta(hours=merged_in_h)
                    if merged_in_h is not None
                    else None
                )
                # Uppercase to match the gold model's state filters
                # (git_metric_observations: prs.state = 'MERGED').
                state = "MERGED" if merged_on else "OPEN"
                merged_naive = None if merged_on is None else merged_on.replace(tzinfo=None)
                pr_added = float(rng.randint(20, 350))
                pr_removed = float(rng.randint(0, 180))
                rows.append((
                    tenant_uuid, pr_id, p.email, author_name,
                    state, created.replace(tzinfo=None),
                    merged_naive, merged_naive,   # closed_on tracks merged_on for merged PRs
                    pr_added, pr_removed,
                    tenant_uuid, "insight_github",
                    version,
                ))
    return bulk_insert(client, "silver", "class_git_pull_requests", cols, rows)


def seed_class_git_file_changes(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> int:
    """One file-change row per commit. Path bucketed so the view's
    code/spec/config classifier finds non-empty bands."""
    truncate(client, "silver", "class_git_file_changes")
    cols = [
        "insight_tenant_id", "commit_hash", "project_key", "repo_slug",
        "tenant_id", "file_path", "lines_added", "lines_removed", "_version",
    ]
    rows: list[tuple[object, ...]] = []
    version = 1
    paths = ["src/main.rs", "src/lib.rs", "tests/test_main.rs", "Cargo.toml"]
    for p in _eligible(roster):
        persona = persona_multiplier(p.uuid)
        weight = TEAM_PROFILES[p.team or ""].weights["github"]
        for d in days_window(days):
            rng = seeded_rng(p.uuid, d, "git.fc")
            mean = 5 * persona * weight * weekday_multiplier(d)
            n_commits = min(poisson(rng, mean), COMMITS_CAP)
            for i in range(n_commits):
                sha = deterministic_uuid("git.commit", p.uuid, d.isoformat(), str(i))[:40]
                sha_clean = sha.replace("-", "")
                # 1-3 file changes per commit
                for j in range(rng.randint(1, 3)):
                    added = rng.randint(2, 180)
                    removed = rng.randint(0, 80)
                    rows.append((
                        tenant_uuid, sha_clean, "insight", "insight/insight",
                        tenant_uuid, paths[j % len(paths)],
                        added, removed, version,
                    ))
    return bulk_insert(client, "silver", "class_git_file_changes", cols, rows)


def generate(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> dict[str, int]:
    _ = clamp  # imported for future use; silence unused warning under strict ruff
    return {
        "silver.class_git_commits":       seed_class_git_commits(client, roster, tenant_uuid, days),
        "silver.class_git_pull_requests": seed_class_git_pull_requests(client, roster, tenant_uuid, days),
        "silver.class_git_file_changes":  seed_class_git_file_changes(client, roster, tenant_uuid, days),
    }
