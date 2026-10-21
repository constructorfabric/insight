"""A bronze row without a tenant stops the build instead of landing under an empty one.

The GitHub models that read a bronze tenant convert it with
`CAST(tenant_id AS String)`, and the identity inputs with `tenant_uuid()`; both
raise on NULL.
"""

from __future__ import annotations

import pytest
from conftest import SOURCE, Warehouse

# The tenant conversion itself, not a later cast of a key the NULL tenant reached.
TENANT_CONVERSION = r"Cannot convert NULL value to non-Nullable type: while executing 'FUNCTION CAST\((__table\d+\.)?tenant_id\b"

LINK_EVENT = {
    "unique_key": f"no-tenant-{SOURCE}-link-1",
    "event_id": "link-1",
    "event_type": "ConnectedEvent",
    "event_at": "2026-01-02T00:00:00Z",
    "item_number": 1,
    "repo_full_name": "example-org/example-repo",
    "actor_login": "example-user",
    "link_target_type": "Issue",
    "link_target_repo_full_name": "example-org/example-repo",
    "link_target_number": 2,
}

# model -> (bronze table, a row of it)
CASES: dict[str, tuple[str, dict[str, object]]] = {
    "github__item_events": (
        "pull_request_timeline_events",
        {**LINK_EVENT, "unique_key": f"no-tenant-{SOURCE}-closed-1", "event_id": "closed-1", "event_type": "ClosedEvent"},
    ),
    "+github__task_links": ("issue_timeline_events", LINK_EVENT),
    "+github__identity_inputs": (
        "commit_authors",
        {
            "unique_key": f"no-tenant-{SOURCE}-author@example.com",
            "repo_full_name": "example-org/example-repo",
            "author_email": "author@example.com",
            "author_login": "example-login",
            "author_id": 101,
            "collected_at": "2026-01-10T00:00:00Z",
        },
    ),
    "+github__unowned_commit_emails": (
        "commits",
        {
            "unique_key": f"no-tenant-{SOURCE}-sha-1",
            "repository": "example-org/example-repo",
            "sha": "sha-1",
            "authored_date": "2026-01-02T00:00:00Z",
            "author_name": "Example Author",
            "author_email": "author@example.com",
        },
    ),
}


@pytest.mark.parametrize("selector", sorted(CASES))
def test_a_bronze_row_without_a_tenant_fails_the_build(case: Warehouse, selector: str) -> None:
    table, row = CASES[selector]
    case.insert(table, None, row, "2026-01-10T00:00:00")

    with pytest.raises(AssertionError, match=TENANT_CONVERSION):
        case.build(selector)
