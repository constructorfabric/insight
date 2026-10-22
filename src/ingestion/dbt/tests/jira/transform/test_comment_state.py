"""A comment is deleted only when a later read of its own issue's list left it out.

Every read of an issue's comment list re-emits each comment still on it, so the
newest extraction among an issue's comments is when that list was last read. A
comment extracted before that read was deleted at the source. The issue-key list
is a stream of its own, and its extraction time is never evidence.
"""

from __future__ import annotations

import uuid
from dataclasses import dataclass
from typing import Any

from conftest import Scenario
from helpers import SOURCE_ID, TENANT_ID

SELECTOR = "+jira__comment_state"

FIRST_READ = "2026-01-15T03:00:00"
NEXT_PAGE_OF_FIRST_READ = "2026-01-15T03:20:00"
LATER_READ = "2026-03-28T03:00:00"


@dataclass(frozen=True)
class CommentCase:
    rule: str
    issue: str
    comment_id: int
    extracted_at: str
    deleted: int


CASES = [
    CommentCase("a newer issue-key row is no evidence", "TST-1", 9001, FIRST_READ, 0),
    CommentCase("a comment a later read re-emitted is live", "TST-2", 9101, LATER_READ, 0),
    CommentCase("a comment a later read left out is deleted", "TST-2", 9102, FIRST_READ, 1),
    CommentCase("pages of one read are one read", "TST-3", 9201, FIRST_READ, 0),
    CommentCase("pages of one read are one read", "TST-3", 9202, NEXT_PAGE_OF_FIRST_READ, 0),
    CommentCase("an issue's only comment has nothing to compare against", "TST-4", 9301, FIRST_READ, 0),
]


def _airbyte(extracted_at: str) -> dict[str, Any]:
    return {
        "_airbyte_raw_id": str(uuid.uuid4()),
        "_airbyte_extracted_at": extracted_at,
        "_airbyte_meta": "{}",
        "_airbyte_generation_id": 0,
    }


def _comment(case: CommentCase) -> dict[str, Any]:
    return {
        **_airbyte(case.extracted_at),
        "id": str(case.comment_id),
        "tenant_id": TENANT_ID,
        "source_id": SOURCE_ID,
        "unique_key": f"{TENANT_ID}-{SOURCE_ID}-{case.comment_id}",
        "comment_id": case.comment_id,
        "id_readable": case.issue,
        "jira_id": case.issue,
        "author_account_id": "author-acct",
        "created": "2026-01-10T09:00:00.000+0000",
        "updated": "2026-01-10T09:00:00.000+0000",
        "body": "{}",
        "collected_at": case.extracted_at,
    }


def _issue_key(key: str, extracted_at: str) -> dict[str, Any]:
    return {
        **_airbyte(extracted_at),
        "id": key,
        "key": key,
        "tenant_id": TENANT_ID,
        "source_id": SOURCE_ID,
        "unique_key": f"{TENANT_ID}-{SOURCE_ID}-{key}",
        "id_readable": key,
        "jira_id": key,
        "updated": "2026-03-27T10:00:00.000+0000",
        "collected_at": extracted_at,
    }


def test_a_comment_is_deleted_only_when_a_later_read_of_its_list_left_it_out(scenario: Scenario) -> None:
    issues = sorted({case.issue for case in CASES})
    scenario.warehouse.insert("bronze_jira.jira_comments", [_comment(case) for case in CASES])
    scenario.warehouse.insert("bronze_jira.jira_issue_keys", [_issue_key(key, LATER_READ) for key in issues])

    scenario.build(SELECTOR)

    rows = scenario.warehouse.rows(
        "SELECT comment_id, is_deleted FROM staging.jira__comment_state FINAL WHERE source_id = {source:String}",
        {"source": SOURCE_ID},
    )
    deleted = {row["comment_id"]: row["is_deleted"] for row in rows}
    for case in CASES:
        assert deleted.get(str(case.comment_id)) == case.deleted, f"{case.rule}: comment {case.comment_id}"
