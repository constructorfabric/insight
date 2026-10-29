#!/usr/bin/env python3
"""Emit the github bronze records one sync delivers, for destination-clickhouse.

Usage: github_records.py <out.jsonl> <round>

Round 1 is the first sync: one repository, its commits, file changes, pull
requests and the accounts behind them, enough to put rows through every
silver and gold model under `tag:github+`. Round 2 is the next sync: the same
records again, extracted later, and each commit carrying ADDED_COLUMN, a
field the connector did not have before (run.sh widens the catalog to match).

The records carry the stamps the connector's `standard_fields` put on every
row, keyed the way the connector keys them, so what the destination writes is
what a real sync writes. `tenant_id` is the rig's canonical tenant (RIG_TENANT in
services.sh, the tenant identity-resolution is configured for); `source_id` is
the fake one in bootstrap-db/connectors-config.yaml.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

NAMESPACE = "bronze_github"
TENANT = "a4d11000-0000-4000-8000-00000000c1a0"
SOURCE = "fake"
REPO_FULL_NAME = "fake/api"
CLONE_URL = f"https://github.com/{REPO_FULL_NAME}.git"
ADDED_COLUMN = "rig_sync_round"
# Round 1 extracted at this instant, round 2 an hour later: the incremental
# staging models take a row only when it is newer than what they hold.
EXTRACTED_AT_MS = {1: 1_760_000_000_000, 2: 1_760_003_600_000}
COLLECTED_AT = "2025-10-09T08:00:00Z"
AUTHORS = (
    ("alice", 1001, "alice@example.com"),
    ("bob", 1002, "bob@example.com"),
    # A noreply address resolves through commit_authors to the account.
    ("carol", 1003, "1003+carol@users.noreply.github.com"),
)
COMMITS = 12
FILES_PER_COMMIT = 2
PULL_REQUESTS = 3
COMMITS_PER_PULL_REQUEST = 2
COUNTS = {
    "repositories": 1,
    "branches": 2,
    "commit_authors": len(AUTHORS),
    "commits": COMMITS,
    "file_changes": COMMITS * FILES_PER_COMMIT,
    "pull_requests": PULL_REQUESTS,
    "pull_request_diff_stats": PULL_REQUESTS,
    "pull_request_commits": PULL_REQUESTS * COMMITS_PER_PULL_REQUEST,
}


def stamp(unique_key: str) -> dict[str, object]:
    return {
        "unique_key": unique_key,
        "tenant_id": TENANT,
        "source_id": SOURCE,
        "data_source": "insight_github",
        "collected_at": COLLECTED_AT,
    }


def sha(i: int) -> str:
    return f"{i:040x}"


def day(i: int, hour: int = 9) -> str:
    return f"2025-09-{1 + i % 28:02d}T{hour:02d}:00:00Z"


def records() -> dict[str, list[dict[str, object]]]:
    out: dict[str, list[dict[str, object]]] = {name: [] for name in COUNTS}
    out["repositories"].append(
        {
            **stamp(f"{TENANT}:{SOURCE}:repo:{REPO_FULL_NAME}"),
            "id": 1,
            "full_name": REPO_FULL_NAME,
            "name": "api",
            "org": "fake",
            "default_branch": "main",
            "archived": False,
            "fork": False,
            "private": True,
            "has_issues": True,
            "has_wiki": False,
            "language": "Rust",
            "size": 1024,
            "clone_url": CLONE_URL,
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": day(27),
            "pushed_at": day(27),
            "description": "the rig's repository",
        }
    )
    for name, is_default in (("main", True), ("feature", False)):
        out["branches"].append(
            {
                **stamp(f"{TENANT}:{SOURCE}:{CLONE_URL}:branch:{name}"),
                "repository": CLONE_URL,
                "name": name,
                "head_sha": sha(COMMITS - 1),
                "head_committed_date": day(COMMITS - 1),
                "is_default": is_default,
            }
        )
    for login, account_id, email in AUTHORS:
        out["commit_authors"].append(
            {
                **stamp(f"{TENANT}:{SOURCE}:{REPO_FULL_NAME}:author:{account_id}"),
                "repo_full_name": REPO_FULL_NAME,
                "author_email": email,
                "author_login": login,
                "author_id": account_id,
                "author_type": "User",
                "sample_sha": sha(0),
                "last_committed_date": day(COMMITS - 1),
            }
        )
    for i in range(COMMITS):
        login, _, email = AUTHORS[i % len(AUTHORS)]
        out["commits"].append(
            {
                **stamp(f"{TENANT}:{SOURCE}:{CLONE_URL.replace(':', '%3A')}:{sha(i)}"),
                "repository": CLONE_URL,
                "sha": sha(i),
                "message": f"change {i}",
                "authored_date": day(i),
                "committed_date": day(i, 10),
                "author_name": login,
                "author_email": email,
                "committer_name": login,
                "committer_email": email,
                "parent_hashes": [sha(i - 1)] if i else [],
                "is_merge": False,
                "additions": 10 + i,
                "deletions": i,
                "changed_files": FILES_PER_COMMIT,
                "is_in_default_branch": True,
                "patch_id": f"patch-{i}",
            }
        )
        for f in range(FILES_PER_COMMIT):
            out["file_changes"].append(
                {
                    **stamp(f"{TENANT}:{SOURCE}:{CLONE_URL}:{sha(i)}:src/file{f}.rs"),
                    "repository": CLONE_URL,
                    "sha": sha(i),
                    "committed_date": day(i, 10),
                    "filename": f"src/file{f}.rs",
                    "status": "modified",
                    "additions": 5 + f,
                    "deletions": f,
                    "changes": 5 + 2 * f,
                    "is_binary": False,
                    "patch_truncated": False,
                    "pre_image_oid": sha(100 + i),
                    "post_image_oid": sha(200 + i),
                }
            )
    for n in range(1, PULL_REQUESTS + 1):
        login, account_id, email = AUTHORS[n % len(AUTHORS)]
        merged = n < PULL_REQUESTS
        out["pull_requests"].append(
            {
                **stamp(f"{TENANT}:{SOURCE}:{REPO_FULL_NAME}:pr:{n}"),
                "id": 500 + n,
                "number": n,
                "repo_full_name": REPO_FULL_NAME,
                "state": "closed" if merged else "open",
                "draft": False,
                "title": f"pull request {n}",
                "author_login": login,
                "author_id": account_id,
                "author_type": "User",
                "head_sha": sha(n * COMMITS_PER_PULL_REQUEST),
                "head_ref": f"feature-{n}",
                "base_ref": "main",
                "merge_commit_sha": sha(n * COMMITS_PER_PULL_REQUEST) if merged else None,
                "created_at": day(n, 8),
                "updated_at": day(n + 1, 13),
                "closed_at": day(n + 1, 13) if merged else None,
                "merged_at": day(n + 1, 13) if merged else None,
                "body": "",
            }
        )
        out["pull_request_diff_stats"].append(
            {
                **stamp(f"{TENANT}:{SOURCE}:{REPO_FULL_NAME}:pr-stats:{n}"),
                "pull_number": n,
                "repo_full_name": REPO_FULL_NAME,
                "additions": 20 * n,
                "deletions": n,
                "changed_files": COMMITS_PER_PULL_REQUEST * FILES_PER_COMMIT,
                "updated_at": day(n + 1, 13),
                "author_email": email,
                "author_login": login,
                "author_id": account_id,
                "review_decision": "APPROVED",
                "total_comments_count": 0,
                "is_cross_repository": False,
            }
        )
        for c in range(COMMITS_PER_PULL_REQUEST):
            i = (n - 1) * COMMITS_PER_PULL_REQUEST + c
            out["pull_request_commits"].append(
                {
                    **stamp(f"{TENANT}:{SOURCE}:{REPO_FULL_NAME}:pr:{n}:{sha(i)}"),
                    "sha": sha(i),
                    "pull_number": n,
                    "repo_full_name": REPO_FULL_NAME,
                    "author_login": login,
                    "author_id": account_id,
                    "author_type": "User",
                    "author_name": login,
                    "author_email": email,
                    "authored_date": day(i),
                    "committer_login": login,
                    "committer_id": account_id,
                    "committer_name": login,
                    "committer_email": email,
                    "committed_date": day(i, 10),
                    "message": f"change {i}",
                    "is_merge": False,
                    "is_verified": True,
                }
            )
    for name, found in out.items():
        assert len(found) == COUNTS[name], (name, len(found), COUNTS[name])
    return out


def messages(sync_round: int) -> list[dict[str, object]]:
    emitted_at = EXTRACTED_AT_MS[sync_round]
    out = []
    for stream, rows in records().items():
        for data in rows:
            if stream == "commits" and sync_round > 1:
                data = {**data, ADDED_COLUMN: sync_round}
            out.append(
                {
                    "type": "RECORD",
                    "record": {"stream": stream, "namespace": NAMESPACE, "emitted_at": emitted_at, "data": data},
                }
            )
    return out


def main(argv: list[str]) -> int:
    out, sync_round = Path(argv[0]), int(argv[1])
    out.write_text("".join(json.dumps(message) + "\n" for message in messages(sync_round)))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
