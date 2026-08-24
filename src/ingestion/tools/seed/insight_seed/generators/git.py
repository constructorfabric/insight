"""
git silver-table generator: commits + pull-requests.

Only the development team produces git activity. Sales / HR / Support
get zero rows here by construction.
"""

from __future__ import annotations

import datetime as _dt
import random
from collections.abc import Sequence
from typing import TYPE_CHECKING

from ..profiles import DEV_LEAD_UUID, TEAM_PROFILES, Person
from .base import (
    clamp,
    days_window,
    deterministic_int,
    deterministic_uuid,
    persona_multiplier,
    poisson,
    seeded_rng,
    weekday_multiplier,
)
from .insert import bulk_insert, truncate

if TYPE_CHECKING:
    import clickhouse_connect.driver.client


# Hard per-person-per-day caps. Generation respects these by
# construction — they aren't validation rules, just upper bounds on
# the Poisson draws so the dataset stays plausible.
COMMITS_CAP = 20
PRS_CAP = 6

# One logical git source for the whole seed. This MUST be written to every
# git silver table: gold/git_metric_observations builds its project and
# repository dimension values with
# `concat(toString(prs.source_id), ':', prs.project_key)`, and `toString`
# of a NULL is NULL, which makes the whole dimension array NULL and fails
# its CAST to a non-nullable Tuple. It is also part of the join key between
# class_git_pull_requests_commits and class_git_commits, and NULL never
# equals NULL in a join.
SOURCE_ID = deterministic_uuid("git.source", "insight_github")
PROJECT_KEY = "insight"
REPO_SLUG = "insight/insight"

# The change_type vocabulary gold/git_metric_observations recognises in its
# change_type_label multiIf; anything else renders as the raw value.
CHANGE_TYPES = ("added", "modified", "renamed", "deleted")

# Deliberately hostile — and clearly synthetic — commit messages for the
# drilldown-export escaping scenario (#1603 scenario 11). The gold evidence
# model surfaces a commit's message as the drilldown "Title" cell, so these
# cover every value class a spreadsheet consumer can mishandle: the four
# formula-prefix bytes ('=', '+', '-', '@') and a value with an embedded tab
# and an embedded newline (which must stay inside one CSV cell). Only the
# FIRST few commits the dev lead generates carry one; every other commit
# keeps the column default, so no row count, metric value, or other
# person's evidence changes.
HOSTILE_COMMIT_MESSAGES = (
    "=SUM(A1:A9) synthetic title",
    "+A1 synthetic title",
    "-2+3 synthetic title",
    "@macro synthetic title",
    "tab\tinside synthetic title",
    "newline\ninside synthetic title",
)


def _eligible(roster: Sequence[Person]) -> list[Person]:
    """Persons whose team profile has any git weight."""
    return [p for p in roster if p.team and TEAM_PROFILES[p.team].weights.get("github", 0) > 0]


def _daily_commit_count(rng: random.Random, mean: float) -> int:
    """The day's commit count, floored at one per person-day.

    The floor keeps every git bucket drillable: the dashboard's trailing
    windows start on whichever calendar day the run lands, so the leading
    partial week bucket can cover a single day — and a zero-commit day
    renders that bucket "—", undrillable. weekday_multiplier still shapes
    the volume.

    One formula on purpose: seed_class_git_pull_requests_commits re-derives
    the day's commit list from the same RNG stream seed_class_git_commits
    draws from, and any divergence — such as only one of them flooring —
    strands that day's PRs without link rows.
    """
    return max(1, min(poisson(rng, mean), COMMITS_CAP))


def seed_class_git_commits(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> int:
    truncate(client, "silver", "class_git_commits")
    cols = [
        "insight_tenant_id",
        "commit_hash",
        "project_key",
        "repo_slug",
        "source_id",
        "tenant_id",
        "author_email",
        "date",
        "is_merge_commit",
        "file_path",
        "lines_added",
        "lines_removed",
        "data_source",
        # Appended after the long-standing columns: the link-parity tests read
        # this table positionally, so a mid-tuple insertion moves their fields.
        "message",
        "_version",
    ]
    rows: list[tuple[object, ...]] = []
    version = 1
    # Row indices of the dev lead's non-merge commits, ascending by date. Gold's
    # evidence model filters merge commits out, so a message on one would never
    # reach the drilldown.
    lead_commits: list[int] = []
    for p in _eligible(roster):
        persona = persona_multiplier(p.uuid)
        weight = TEAM_PROFILES[p.team or ""].weights["github"]
        for d in days_window(days):
            rng = seeded_rng(p.uuid, d, "git.commits")
            mean = 5 * persona * weight * weekday_multiplier(d)
            n_commits = _daily_commit_count(rng, mean)
            for i in range(n_commits):
                sha = deterministic_uuid("git.commit", p.uuid, d.isoformat(), str(i))[:40]
                is_merge = 1 if rng.random() < 0.05 else 0
                # LOC per commit capped at ≤200 by construction.
                added = float(rng.randint(2, 180))
                removed = float(rng.randint(0, 80))
                if p.uuid == DEV_LEAD_UUID and not is_merge:
                    lead_commits.append(len(rows))
                rows.append(
                    (
                        tenant_uuid,
                        sha.replace("-", ""),
                        PROJECT_KEY,
                        REPO_SLUG,
                        SOURCE_ID,
                        tenant_uuid,
                        p.email,
                        d,
                        is_merge,
                        "src/main.rs",
                        added,
                        removed,
                        "insight_github",
                        "",
                        version,
                    )
                )

    # The most recent of those commits, not the earliest: a suite asking about
    # "the seeded period" asks about the tail the API will answer for, and a
    # window wider than that cap would leave titles dealt to the oldest days
    # unreachable.
    message_at = cols.index("message")
    for offset, hostile in enumerate(reversed(HOSTILE_COMMIT_MESSAGES), start=1):
        if offset > len(lead_commits):
            break
        index = lead_commits[-offset]
        row = list(rows[index])
        row[message_at] = hostile
        rows[index] = tuple(row)

    return bulk_insert(client, "silver", "class_git_commits", cols, rows)


def seed_class_git_pull_requests(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> int:
    truncate(client, "silver", "class_git_pull_requests")
    cols = [
        "insight_tenant_id",
        "pr_id",
        "author_email",
        "author_name",
        "state",
        "created_on",
        "merged_on",
        "closed_on",
        "lines_added",
        "lines_removed",
        "tenant_id",
        "data_source",
        "_version",
        # gold/git_metric_observations puts `destination_branch` into a
        # dimension tuple whose value slot is non-nullable, and guards only
        # the empty string (`if(x = '', '__unknown__', x)`), not NULL. Emit
        # both branch names explicitly, as a real git connector would,
        # rather than relying on the column default.
        "source_branch",
        "destination_branch",
        "source_id",
        "project_key",
        "repo_slug",
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
                    d,
                    _dt.time(9 + rng.randint(0, 8), rng.randint(0, 59), tzinfo=_dt.UTC),
                )
                merged_in_h = rng.randint(1, 72) if rng.random() < 0.85 else None
                merged_on = (
                    created + _dt.timedelta(hours=merged_in_h) if merged_in_h is not None else None
                )
                # Uppercase to match the gold model's state filters
                # (git_metric_observations: prs.state = 'MERGED').
                state = "MERGED" if merged_on else "OPEN"
                merged_naive = None if merged_on is None else merged_on.replace(tzinfo=None)
                pr_added = float(rng.randint(20, 350))
                pr_removed = float(rng.randint(0, 180))
                rows.append(
                    (
                        tenant_uuid,
                        pr_id,
                        p.email,
                        author_name,
                        state,
                        created.replace(tzinfo=None),
                        merged_naive,
                        merged_naive,  # closed_on tracks merged_on for merged PRs
                        pr_added,
                        pr_removed,
                        tenant_uuid,
                        "insight_github",
                        version,
                        f"feature/pr-{pr_id}",
                        "main",
                        SOURCE_ID,
                        PROJECT_KEY,
                        REPO_SLUG,
                    )
                )
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
        "insight_tenant_id",
        "commit_hash",
        "project_key",
        "repo_slug",
        "source_id",
        "tenant_id",
        "file_path",
        "lines_added",
        "lines_removed",
        "_version",
        # gold/git_metric_observations turns these into the `file_extension`
        # and `change_type` dimensions on code_lines_added / lines_added /
        # lines_removed, mapping '' -> '__unknown__'. Left unwritten they
        # default to '', so both dimensions collapse to a single "Unknown"
        # bucket — the same output as seeding nothing. A real connector
        # derives them from the filename and the API status, so do the same.
        "file_extension",
        "change_type",
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
                    # Offset by the commit index too. `j` only ever reaches
                    # 0-2, so indexing on `j` alone never selects the 4th
                    # path — leaving gold's `file_extension` dimension at a
                    # single bucket and its code/spec/config `category`
                    # classifier without a config band, which is exactly what
                    # this function's docstring says the path list is for.
                    path = paths[(i + j) % len(paths)]
                    extension = path.rsplit(".", 1)[-1] if "." in path else ""
                    change_type = CHANGE_TYPES[
                        deterministic_int("git.fc.change", sha_clean, str(j)) % len(CHANGE_TYPES)
                    ]
                    rows.append(
                        (
                            tenant_uuid,
                            sha_clean,
                            PROJECT_KEY,
                            REPO_SLUG,
                            SOURCE_ID,
                            tenant_uuid,
                            path,
                            added,
                            removed,
                            version,
                            extension,
                            change_type,
                        )
                    )
    return bulk_insert(client, "silver", "class_git_file_changes", cols, rows)


def seed_class_git_pull_requests_commits(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> int:
    """PR -> commit links.

    gold/git_metric_observations derives per-PR author attribution from this
    table (its `pr_commit_emails` CTE INNER JOINs it against
    class_git_commits, then LEFT JOINs the result onto the PRs). Without it
    the CTE is empty, every PR row misses the LEFT JOIN, and the model's
    dimension CAST fails on the resulting NULLs — so the table is not
    optional for a stand that expects git metrics.

    Links are reconstructed from the same seeded RNG streams the PR and
    commit generators use, so a PR's commits are that author's commits from
    the same day. Deterministic across runs by construction.
    """
    truncate(client, "silver", "class_git_pull_requests_commits")
    cols = [
        "tenant_id",
        "source_id",
        "project_key",
        "repo_slug",
        "pr_id",
        "commit_hash",
        "commit_order",
        "data_source",
        "_version",
    ]
    rows: list[tuple[object, ...]] = []
    version = 1
    for p in _eligible(roster):
        persona = persona_multiplier(p.uuid)
        weight = TEAM_PROFILES[p.team or ""].weights["github"]
        for d in days_window(days):
            # Re-derive the day's commit hashes exactly as
            # seed_class_git_commits does (same salt, same draw order).
            crng = seeded_rng(p.uuid, d, "git.commits")
            cmean = 5 * persona * weight * weekday_multiplier(d)
            n_commits = _daily_commit_count(crng, cmean)
            hashes = [
                deterministic_uuid("git.commit", p.uuid, d.isoformat(), str(i))[:40].replace(
                    "-", ""
                )
                for i in range(n_commits)
            ]
            # Re-derive the day's PR ids the same way.
            prng = seeded_rng(p.uuid, d, "git.prs")
            pmean = 0.8 * persona * weight * weekday_multiplier(d)
            n_prs = min(poisson(prng, pmean), PRS_CAP)
            for i in range(n_prs):
                pr_id = deterministic_int("git.pr", p.uuid, d.isoformat(), str(i))
                # Deal the day's commits round-robin across the day's PRs.
                # A day with fewer commits than PRs wraps instead of leaving
                # the tail PRs linkless: gold's pr_commit_emails derives a
                # PR's author attribution from these links, so a linkless PR
                # NULLs its dimensions, while a commit shared by two PRs is
                # ordinary git.
                linked = hashes[i::n_prs] or [hashes[i % n_commits]]
                for order, commit_hash in enumerate(linked):
                    rows.append(
                        (
                            tenant_uuid,
                            SOURCE_ID,
                            PROJECT_KEY,
                            REPO_SLUG,
                            pr_id,
                            commit_hash,
                            order,
                            "insight_github",
                            version,
                        )
                    )
    return bulk_insert(client, "silver", "class_git_pull_requests_commits", cols, rows)


def generate(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> dict[str, int]:
    _ = clamp  # imported for future use; silence unused warning under strict ruff
    return {
        "silver.class_git_commits": seed_class_git_commits(client, roster, tenant_uuid, days),
        "silver.class_git_pull_requests": seed_class_git_pull_requests(
            client, roster, tenant_uuid, days
        ),
        "silver.class_git_file_changes": seed_class_git_file_changes(
            client, roster, tenant_uuid, days
        ),
        "silver.class_git_pull_requests_commits": seed_class_git_pull_requests_commits(
            client, roster, tenant_uuid, days
        ),
    }
