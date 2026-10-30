"""A bronze row without a tenant builds, and the tenant tests on its models report it.

A key reads the tenant through `ifNull(tenant_id, '')`, so a missing one never makes
a key Nullable; `not_null` and `not_empty` on each model's tenant are the guard.
"""

from __future__ import annotations

import pytest
from conftest import SOURCE, TENANT, Warehouse


def tenant_tests(model: str) -> tuple[str, str]:
    return f"not_null_{model}_tenant_id", f"not_empty_{model}_tenant_id"


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

CASES: dict[str, tuple[str, dict[str, object]]] = {
    "github__item_events": (
        "pull_request_timeline_events",
        {
            **LINK_EVENT,
            "unique_key": f"no-tenant-{SOURCE}-closed-1",
            "event_id": "closed-1",
            "event_type": "ClosedEvent",
        },
    ),
    "+github__task_links": ("issue_timeline_events", LINK_EVENT),
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


REPORTED_BY: dict[str, tuple[str, str]] = {
    "github__item_events": ("github__item_events", "not_empty_github__item_events_tenant_id"),
    "+github__task_links": ("github__issue_link_events", "not_null_github__issue_link_events_tenant_id"),
    "+github__unowned_commit_emails": (
        "github__unowned_commit_emails",
        "not_null_github__unowned_commit_emails_tenant_id",
    ),
}


def test_the_tenant_tests_pass_over_rows_that_carry_a_tenant(case: Warehouse) -> None:
    for table, row in CASES.values():
        case.insert(table, TENANT, row, "2026-01-10T00:00:00")
    for selector in CASES:
        case.build(selector)

    every_test = [name for model, _ in REPORTED_BY.values() for name in tenant_tests(model)]
    assert case.failing_tests(every_test) == set(), "no tenant test should fail while every row carries a tenant"


@pytest.mark.parametrize("selector", sorted(CASES))
def test_a_bronze_row_without_a_tenant_builds_and_the_tenant_tests_report_it(case: Warehouse, selector: str) -> None:
    table, row = CASES[selector]
    model, reported = REPORTED_BY[selector]
    case.insert(table, None, row, "2026-01-10T00:00:00")
    case.build(selector)

    failing = case.failing_tests(tenant_tests(model))
    assert reported in failing, f"{model} should report the {table} row without a tenant: {sorted(failing)}"


IDENTITY_AUTHOR = {
    "unique_key": f"no-tenant-{SOURCE}-author@example.com",
    "repo_full_name": "example-org/example-repo",
    "author_email": "author@example.com",
    "author_login": "example-login",
    "author_id": 101,
    "collected_at": "2026-01-10T00:00:00Z",
}


def test_the_identity_inputs_refuse_a_bronze_row_without_a_tenant(case: Warehouse) -> None:
    case.insert("commit_authors", None, IDENTITY_AUTHOR, "2026-01-10T00:00:00")

    with pytest.raises(AssertionError, match="Cannot parse UUID"):
        case.build("+github__identity_inputs")
