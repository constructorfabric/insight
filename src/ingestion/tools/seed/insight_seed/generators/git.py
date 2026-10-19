"""
git silver-table generator: commits + pull-requests.

Only the development team produces git activity. Sales / HR / Support
get zero rows here by construction. RNG draws for the commit and
pull-request skeleton live in `git_history` — this module shapes silver
rows from what that builder produced.
"""

from __future__ import annotations

import datetime as _dt
from collections.abc import Sequence
from typing import TYPE_CHECKING

from ..profiles import DEV_LEAD_UUID, Person
from . import git_history
from .base import (
    anchor_datetime,
    deterministic_int,
    deterministic_uuid,
    seeded_rng,
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


AIRBYTE_COLS = [
    "_airbyte_raw_id",
    "_airbyte_extracted_at",
    "_airbyte_meta",
    "_airbyte_generation_id",
]
IDENTITY_COLS = ["unique_key", "tenant_id", "source_id", "data_source", "collected_at"]


def _positioned(cols: list[str], fields: dict[str, object]) -> tuple[object, ...]:
    return tuple(fields.get(col) for col in cols)


def _envelope(tenant_uuid: str) -> dict[str, object]:
    read_at = anchor_datetime()
    return {
        "_airbyte_extracted_at": read_at,
        "_airbyte_meta": "{}",
        "_airbyte_generation_id": 0,
        "tenant_id": tenant_uuid,
        "source_id": SOURCE_ID,
        "data_source": "insight_github",
        "collected_at": read_at.isoformat(),
    }


def _login(person: Person) -> str:
    return person.email.split("@", 1)[0]


def _author_name(person: Person) -> str:
    return _login(person).replace("_", " ").title()


def _sha(commit_hash: str) -> str:
    return commit_hash.replace("-", "")[:40]


def _commit_message(person: Person, index: int, hostile: str | None) -> str:
    return hostile if hostile else f"Change {index} by {_login(person)}"


def _hostile_by_sha(history: Sequence[DayHistory]) -> dict[str, str]:
    """The dev lead's most recent non-merge commits carry the hostile titles.

    The most recent, not the earliest: a suite asking about "the seeded period"
    asks about the tail the API will answer for, and a window wider than that
    cap would leave titles dealt to the oldest days unreachable. Gold's evidence
    model filters merge commits out, so a message on one would never reach the
    drilldown.
    """
    lead_shas = [
        _sha(commit.hash)
        for day in history
        if day.person.uuid == DEV_LEAD_UUID
        for commit in day.commits
        if not commit.is_merge
    ]
    return dict(
        zip(
            lead_shas[-len(HOSTILE_COMMIT_MESSAGES) :],
            HOSTILE_COMMIT_MESSAGES[-len(lead_shas) :],
            strict=False,
        )
    )


def seed_commits(
    client: clickhouse_connect.driver.client.Client,
    history: Sequence[DayHistory],
    tenant_uuid: str,
) -> int:
    truncate(client, "bronze_github", "commits")
    cols = [
        *AIRBYTE_COLS,
        *IDENTITY_COLS,
        "repository",
        "sha",
        "message",
        "authored_date",
        "committed_date",
        "author_name",
        "author_email",
        "committer_name",
        "committer_email",
        "parent_hashes",
        "is_merge",
        "additions",
        "deletions",
        "changed_files",
        "is_in_default_branch",
    ]
    hostile = _hostile_by_sha(history)
    rows: list[tuple[object, ...]] = []
    for day in history:
        at = _dt.datetime.combine(day.date, _dt.time(12, tzinfo=_dt.UTC))
        for index, commit in enumerate(day.commits):
            sha = _sha(commit.hash)
            rows.append(
                _positioned(
                    cols,
                    {
                        **_envelope(tenant_uuid),
                        "_airbyte_raw_id": deterministic_uuid("git.commit.raw", sha),
                        "unique_key": f"{tenant_uuid}:{SOURCE_ID}:{sha}",
                        "repository": REPO_SLUG,
                        "sha": sha,
                        "message": _commit_message(day.person, index, hostile.get(sha)),
                        "authored_date": at.isoformat(),
                        "committed_date": at.isoformat(),
                        "author_name": _author_name(day.person),
                        "author_email": day.person.email,
                        "committer_name": _author_name(day.person),
                        "committer_email": day.person.email,
                        # Two parents is what makes a commit a merge to any
                        # reader that counts them rather than trusting a flag.
                        "parent_hashes": "a b" if commit.is_merge else "a",
                        "is_merge": commit.is_merge,
                        "additions": int(commit.lines_added),
                        "deletions": int(commit.lines_removed),
                        "changed_files": 1,
                        "is_in_default_branch": True,
                    },
                )
            )
    return bulk_insert(client, "bronze_github", "commits", cols, rows)


def seed_file_changes(
    client: clickhouse_connect.driver.client.Client,
    history: Sequence[DayHistory],
    tenant_uuid: str,
) -> int:
    """One to three files per commit, bucketed so gold's code/spec/config
    classifier and its file_extension dimension find non-empty bands.

    Hung off the history's own commits: bronze file changes join their commit
    by sha, and the previous generator drew a separate set of hashes that no
    commit row ever carried.
    """
    truncate(client, "bronze_github", "file_changes")
    cols = [
        *AIRBYTE_COLS,
        *IDENTITY_COLS,
        "repository",
        "sha",
        "committed_date",
        "filename",
        "status",
        "additions",
        "deletions",
        "changes",
        "is_binary",
    ]
    paths = ["src/main.rs", "src/lib.rs", "tests/test_main.rs", "Cargo.toml"]
    rows: list[tuple[object, ...]] = []
    for day in history:
        at = _dt.datetime.combine(day.date, _dt.time(12, tzinfo=_dt.UTC))
        rng = seeded_rng(day.person.uuid, day.date, "git.fc")
        for index, commit in enumerate(day.commits):
            sha = _sha(commit.hash)
            for position in range(rng.randint(1, 3)):
                added = rng.randint(2, 180)
                removed = rng.randint(0, 80)
                # Offset by the commit index too: `position` only ever reaches
                # 0-2, so indexing on it alone never selects the 4th path,
                # leaving the classifier without a config band.
                path = paths[(index + position) % len(paths)]
                status = CHANGE_TYPES[
                    deterministic_int("git.fc.change", sha, str(position)) % len(CHANGE_TYPES)
                ]
                rows.append(
                    _positioned(
                        cols,
                        {
                            **_envelope(tenant_uuid),
                            "_airbyte_raw_id": deterministic_uuid("git.fc.raw", sha, str(position)),
                            "unique_key": f"{tenant_uuid}:{SOURCE_ID}:{sha}:{path}",
                            "repository": REPO_SLUG,
                            "sha": sha,
                            "committed_date": at.isoformat(),
                            "filename": path,
                            "status": status,
                            "additions": added,
                            "deletions": removed,
                            "changes": added + removed,
                            "is_binary": False,
                        },
                    )
                )
    return bulk_insert(client, "bronze_github", "file_changes", cols, rows)


def seed_pull_requests(
    client: clickhouse_connect.driver.client.Client,
    history: Sequence[DayHistory],
    tenant_uuid: str,
) -> dict[str, int]:
    """The requests, their commits and the diff stats gold reads sizes from."""
    truncate(client, "bronze_github", "pull_requests")
    truncate(client, "bronze_github", "pull_request_commits")
    truncate(client, "bronze_github", "pull_request_diff_stats")
    pr_cols = [
        *AIRBYTE_COLS,
        *IDENTITY_COLS,
        "id",
        "number",
        "repo_full_name",
        "state",
        "draft",
        "title",
        "author_login",
        "author_id",
        "author_type",
        "head_sha",
        "head_ref",
        "base_ref",
        "merge_commit_sha",
        "created_at",
        "updated_at",
        "closed_at",
        "merged_at",
    ]
    link_cols = [
        *AIRBYTE_COLS,
        *IDENTITY_COLS,
        "sha",
        "pull_number",
        "repo_full_name",
        "author_login",
        "author_name",
        "author_email",
        "authored_date",
        "committed_date",
        "message",
        "is_merge",
    ]
    stat_cols = [
        *AIRBYTE_COLS,
        *IDENTITY_COLS,
        "pull_number",
        "repo_full_name",
        "additions",
        "deletions",
        "changed_files",
        "updated_at",
        "author_email",
        "author_login",
    ]
    prs: list[tuple[object, ...]] = []
    links: list[tuple[object, ...]] = []
    stats: list[tuple[object, ...]] = []

    for day in history:
        if not day.prs:
            continue
        shas = [_sha(c.hash) for c in day.commits]
        at = _dt.datetime.combine(day.date, _dt.time(12, tzinfo=_dt.UTC))
        for index, pr in enumerate(day.prs):
            merged = pr.merged_on.isoformat() if pr.merged_on else None
            # Deal the day's commits round-robin across the day's requests. A
            # day with fewer commits than requests wraps instead of leaving the
            # tail linkless: gold derives a request's author attribution from
            # these links, while a commit on two requests is ordinary git.
            linked = shas[index :: len(day.prs)] or ([shas[index % len(shas)]] if shas else [])
            prs.append(
                _positioned(
                    pr_cols,
                    {
                        **_envelope(tenant_uuid),
                        "_airbyte_raw_id": deterministic_uuid("git.pr.raw", str(pr.pr_id)),
                        "unique_key": f"{tenant_uuid}:{SOURCE_ID}:{pr.pr_id}",
                        "id": pr.pr_id,
                        "number": pr.pr_id,
                        "repo_full_name": REPO_SLUG,
                        # Lower-case as GitHub reports it; the staging model
                        # raises it to the MERGED the gold filters read.
                        "state": "closed" if pr.merged_on else "open",
                        "draft": False,
                        "title": f"Change {pr.pr_id}",
                        "author_login": _login(day.person),
                        "author_id": deterministic_int("git.login", day.person.email),
                        "author_type": "User",
                        "head_sha": linked[0] if linked else "",
                        "head_ref": f"feature/pr-{pr.pr_id}",
                        "base_ref": "main",
                        "merge_commit_sha": linked[0] if linked and pr.merged_on else "",
                        "created_at": pr.created.isoformat(),
                        "updated_at": merged or pr.created.isoformat(),
                        "closed_at": merged,
                        "merged_at": merged,
                    },
                )
            )
            stats.append(
                _positioned(
                    stat_cols,
                    {
                        **_envelope(tenant_uuid),
                        "_airbyte_raw_id": deterministic_uuid("git.prstat.raw", str(pr.pr_id)),
                        "unique_key": f"{tenant_uuid}:{SOURCE_ID}:stat:{pr.pr_id}",
                        "pull_number": pr.pr_id,
                        "repo_full_name": REPO_SLUG,
                        "additions": int(pr.lines_added),
                        "deletions": int(pr.lines_removed),
                        "changed_files": max(1, len(linked)),
                        "updated_at": merged or pr.created.isoformat(),
                        "author_email": day.person.email,
                        "author_login": _login(day.person),
                    },
                )
            )
            for sha in linked:
                links.append(
                    _positioned(
                        link_cols,
                        {
                            **_envelope(tenant_uuid),
                            "_airbyte_raw_id": deterministic_uuid(
                                "git.prc.raw", str(pr.pr_id), sha
                            ),
                            "unique_key": f"{tenant_uuid}:{SOURCE_ID}:{pr.pr_id}:{sha}",
                            "sha": sha,
                            "pull_number": pr.pr_id,
                            "repo_full_name": REPO_SLUG,
                            "author_login": _login(day.person),
                            "author_name": _author_name(day.person),
                            "author_email": day.person.email,
                            "authored_date": at.isoformat(),
                            "committed_date": at.isoformat(),
                            "message": f"Change on {pr.pr_id}",
                            "is_merge": False,
                        },
                    )
                )

    return {
        "bronze_github.pull_requests": bulk_insert(
            client, "bronze_github", "pull_requests", pr_cols, prs
        ),
        "bronze_github.pull_request_commits": bulk_insert(
            client, "bronze_github", "pull_request_commits", link_cols, links
        ),
        "bronze_github.pull_request_diff_stats": bulk_insert(
            client, "bronze_github", "pull_request_diff_stats", stat_cols, stats
        ),
    }


def seed_commit_authors(
    client: clickhouse_connect.driver.client.Client,
    history: Sequence[DayHistory],
    tenant_uuid: str,
) -> int:
    """The email-to-login directory `github__account_emails` resolves through."""
    truncate(client, "bronze_github", "commit_authors")
    cols = [
        *AIRBYTE_COLS,
        *IDENTITY_COLS,
        "repo_full_name",
        "author_email",
        "author_login",
        "author_id",
        "author_type",
        "sample_sha",
        "last_committed_date",
    ]
    seen: dict[str, tuple[Person, str, _dt.date]] = {}
    for day in history:
        for commit in day.commits:
            known = seen.get(day.person.email)
            if known is None or known[2] < day.date:
                seen[day.person.email] = (day.person, _sha(commit.hash), day.date)
    rows = [
        _positioned(
            cols,
            {
                **_envelope(tenant_uuid),
                "_airbyte_raw_id": deterministic_uuid("git.author.raw", email),
                "unique_key": f"{tenant_uuid}:{SOURCE_ID}:{email}",
                "repo_full_name": REPO_SLUG,
                "author_email": email,
                "author_login": _login(person),
                "author_id": deterministic_int("git.login", email),
                "author_type": "User",
                "sample_sha": sample,
                "last_committed_date": _dt.datetime.combine(
                    last, _dt.time(12, tzinfo=_dt.UTC)
                ).isoformat(),
            },
        )
        for email, (person, sample, last) in sorted(seen.items())
    ]
    return bulk_insert(client, "bronze_github", "commit_authors", cols, rows)


def generate(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
    history: Sequence[DayHistory] | None = None,
) -> dict[str, int]:
    truncate(client, "staging", "github__commits")
    truncate(client, "staging", "github__file_changes")
    truncate(client, "staging", "github__pull_requests")
    truncate(client, "staging", "github__pull_requests_commits")
    truncate(client, "silver", "class_git_commits")
    truncate(client, "silver", "class_git_file_changes")
    truncate(client, "silver", "class_git_pull_requests")
    truncate(client, "silver", "class_git_pull_requests_commits")

    if history is None:
        history = build_history(roster, days)

    return {
        "bronze_github.commits": seed_commits(client, history, tenant_uuid),
        "bronze_github.file_changes": seed_file_changes(client, history, tenant_uuid),
        "bronze_github.commit_authors": seed_commit_authors(client, history, tenant_uuid),
        **seed_pull_requests(client, history, tenant_uuid),
    }
