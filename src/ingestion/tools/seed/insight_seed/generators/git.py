"""
git silver-table generator: commits + pull-requests.

Only the development team produces git activity. Sales / HR / Support
get zero rows here by construction. RNG draws for the commit and
pull-request skeleton live in `git_history` — this module shapes silver
rows from what that builder produced.
"""

from __future__ import annotations

from collections.abc import Sequence
from typing import TYPE_CHECKING

from ..profiles import DEV_LEAD_UUID, TEAM_PROFILES, Person
from . import git_history
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
from .git_history import DayHistory, build_history
from .insert import bulk_insert, truncate

if TYPE_CHECKING:
    import clickhouse_connect.driver.client

# WORKAROUND: assignment, not import — mypy's no_implicit_reexport would
# otherwise hide these from test_git_links.py's `git.<name>` access.
COMMITS_CAP = git_history.COMMITS_CAP
PRS_CAP = git_history.PRS_CAP
_eligible = git_history.eligible


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


def seed_class_git_commits(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
    history: Sequence[DayHistory] | None = None,
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
    if history is None:
        history = build_history(roster, days)
    for day in history:
        for commit in day.commits:
            if day.person.uuid == DEV_LEAD_UUID and not commit.is_merge:
                lead_commits.append(len(rows))
            rows.append(
                (
                    tenant_uuid,
                    commit.hash,
                    PROJECT_KEY,
                    REPO_SLUG,
                    SOURCE_ID,
                    tenant_uuid,
                    day.person.email,
                    day.date,
                    1 if commit.is_merge else 0,
                    "src/main.rs",
                    commit.lines_added,
                    commit.lines_removed,
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
    history: Sequence[DayHistory] | None = None,
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
    if history is None:
        history = build_history(roster, days)
    for day in history:
        author_name = day.person.email.split("@", 1)[0].replace("_", " ").title()
        for pr in day.prs:
            # Uppercase to match the gold model's state filters
            # (git_metric_observations: prs.state = 'MERGED').
            state = "MERGED" if pr.merged_on else "OPEN"
            merged_naive = None if pr.merged_on is None else pr.merged_on.replace(tzinfo=None)
            rows.append(
                (
                    tenant_uuid,
                    pr.pr_id,
                    day.person.email,
                    author_name,
                    state,
                    pr.created.replace(tzinfo=None),
                    merged_naive,
                    merged_naive,  # closed_on tracks merged_on for merged PRs
                    pr.lines_added,
                    pr.lines_removed,
                    tenant_uuid,
                    "insight_github",
                    version,
                    f"feature/pr-{pr.pr_id}",
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
    history: Sequence[DayHistory] | None = None,
) -> int:
    """PR -> commit links.

    gold/git_metric_observations derives per-PR author attribution from this
    table (its `pr_commit_emails` CTE INNER JOINs it against
    class_git_commits, then LEFT JOINs the result onto the PRs). Without it
    the CTE is empty, every PR row misses the LEFT JOIN, and the model's
    dimension CAST fails on the resulting NULLs — so the table is not
    optional for a stand that expects git metrics.

    Links are dealt round-robin from the same `DayHistory.commits` and
    `.prs` that `seed_class_git_commits` and `seed_class_git_pull_requests`
    themselves render, so a PR's commits are that author's commits from the
    same day. Deterministic across runs by construction.
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
    if history is None:
        history = build_history(roster, days)
    for day in history:
        if not day.prs:
            continue
        hashes = [c.hash for c in day.commits]
        for i, pr in enumerate(day.prs):
            # Deal the day's commits round-robin across the day's PRs. A day
            # with fewer commits than PRs wraps instead of leaving the tail
            # PRs linkless: gold's pr_commit_emails derives a PR's author
            # attribution from these links, so a linkless PR NULLs its
            # dimensions, while a commit shared by two PRs is ordinary git.
            linked = hashes[i :: len(day.prs)] or [hashes[i % len(hashes)]]
            for order, commit_hash in enumerate(linked):
                rows.append(
                    (
                        tenant_uuid,
                        SOURCE_ID,
                        PROJECT_KEY,
                        REPO_SLUG,
                        pr.pr_id,
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
    history: Sequence[DayHistory],
) -> dict[str, int]:
    _ = clamp  # imported for future use; silence unused warning under strict ruff
    return {
        "silver.class_git_commits": seed_class_git_commits(
            client, roster, tenant_uuid, days, history
        ),
        "silver.class_git_pull_requests": seed_class_git_pull_requests(
            client, roster, tenant_uuid, days, history
        ),
        "silver.class_git_file_changes": seed_class_git_file_changes(
            client, roster, tenant_uuid, days
        ),
        "silver.class_git_pull_requests_commits": seed_class_git_pull_requests_commits(
            client, roster, tenant_uuid, days, history
        ),
    }
