"""Two tenants that share a source id stay apart in the GitHub staging models.

A source id names a connector instance inside one tenant, so two tenants can hold
the very same source id with different contents.
"""

from __future__ import annotations

from conftest import OTHER_TENANT, SOURCE, TENANT, Warehouse


def _timeline_event(case: Warehouse, tenant: str, event_id: str, extracted: str) -> None:
    case.insert(
        "pull_request_timeline_events",
        tenant,
        {
            "unique_key": f"{tenant}-{SOURCE}-{event_id}",
            "event_id": event_id,
            "event_type": "ClosedEvent",
            "event_at": "2026-01-02T00:00:00Z",
            "item_number": 1,
            "repo_full_name": "example-org/example-repo",
            "actor_login": "example-user",
        },
        extracted,
    )


def _commit(case: Warehouse, tenant: str, sha: str, email: str) -> None:
    case.insert(
        "commits",
        tenant,
        {
            "unique_key": f"{tenant}-{SOURCE}-{sha}",
            "repository": "example-org/example-repo",
            "sha": sha,
            "authored_date": "2026-01-02T00:00:00Z",
            "author_name": "Example Author",
            "author_email": email,
        },
        "2026-01-10T00:00:00",
    )


def _account(case: Warehouse, tenant: str, email: str, account_id: int) -> None:
    case.insert(
        "commit_authors",
        tenant,
        {
            "unique_key": f"{tenant}-{SOURCE}-{email}",
            "repo_full_name": "example-org/example-repo",
            "author_email": email,
            "author_login": f"example-login-{account_id}",
            "author_id": account_id,
            "collected_at": "2026-01-10T00:00:00Z",
        },
        "2026-01-10T00:00:00",
    )


def test_a_tenant_still_picks_up_events_older_than_another_tenants_watermark(case: Warehouse) -> None:
    _timeline_event(case, TENANT, "newer", "2026-01-10T00:00:00")
    case.build("github__item_events")
    _timeline_event(case, OTHER_TENANT, "older", "2026-01-05T00:00:00")
    case.build("github__item_events")

    found = case.rows(
        f"SELECT tenant_id, event_id FROM staging.github__item_events WHERE source_id = '{SOURCE}' ORDER BY tenant_id"
    )

    assert found == [{"tenant_id": OTHER_TENANT, "event_id": "older"}, {"tenant_id": TENANT, "event_id": "newer"}], (
        found
    )


def test_an_email_owned_only_in_another_tenant_is_still_unowned_here(case: Warehouse) -> None:
    _commit(case, TENANT, "sha-1", "shared@example.com")
    _commit(case, TENANT, "sha-2", "owned@example.com")
    _account(case, TENANT, "owned@example.com", 101)
    _commit(case, OTHER_TENANT, "sha-1", "shared@example.com")
    _account(case, OTHER_TENANT, "shared@example.com", 202)
    case.build("+github__unowned_commit_emails")

    found = case.rows(
        "SELECT tenant_id, email FROM staging.github__unowned_commit_emails"
        f" WHERE source_id = '{SOURCE}' ORDER BY tenant_id, email"
    )

    assert found == [{"tenant_id": TENANT, "email": "shared@example.com"}], found
