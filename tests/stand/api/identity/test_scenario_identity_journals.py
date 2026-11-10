from __future__ import annotations

from typing import Literal

import pytest
from insight_stand import ApiClient, ApiResponse, Manifest, PersonaSession, identity_path
from pydantic import BaseModel, Field

from ..schemas import (
    PROBLEM_CONTENT_TYPE,
    AccountSearchResponse,
    AttentionResponse,
    Operation,
    PersonListResponse,
    ProblemDocument,
    SeedOperationList,
    SyncOperationList,
)
from ..schemas.identity import PersonsSyncOperationResponse

SEED_JOURNAL = identity_path("/v1/persons-seed")
SYNC_JOURNAL = identity_path("/v1/persons-sync")


class SeedSummary(BaseModel):
    accounts_read: int = Field(ge=0)
    accounts_reused_known: int = Field(ge=0)
    accounts_linked_by_email: int = Field(ge=0)
    accounts_minted_new: int = Field(ge=0)
    accounts_minted_from_roster: int = Field(ge=0)
    accounts_skipped_closed: int = Field(ge=0)
    accounts_skipped_no_email: int = Field(ge=0)
    accounts_skipped_no_source_id: int = Field(ge=0)
    skipped_contested_email: int = Field(ge=0)
    skipped_excluded: int = Field(ge=0)
    known_binding_conflicts: int = Field(ge=0)
    operator_settled_groups: int = Field(ge=0)
    observations_inserted: int = Field(ge=0)
    org_chart_rows_rebuilt: int = Field(ge=0)
    people_opened: int = Field(ge=0)
    people_closed: int = Field(ge=0)
    people_unchanged: int = Field(ge=0)


class Cursor(BaseModel):
    next_cursor: str | None = None


class SyncSummary(BaseModel):
    outcome: Literal["published", "already_current"]
    max_id: int | None = None
    rows: int | None = None
    max_created_at: str | None = None
    synced_at: str | None = None


def _latest_completed_seed(client: ApiClient) -> Operation:
    response = client.get(SEED_JOURNAL)
    assert response.status_code == 200, f"{response.status_code} {response.text[:300]}"
    completed = [op for op in response.parse(SeedOperationList).items if op.status == "completed"]
    assert completed, "a seeded stand has no completed persons-seed operation"
    return max(completed, key=lambda op: op.started_at)


def _completed_syncs(client: ApiClient) -> list[PersonsSyncOperationResponse]:
    response = client.get(SYNC_JOURNAL)
    assert response.status_code == 200, f"{response.status_code} {response.text[:300]}"
    return sorted(
        (op for op in response.parse(SyncOperationList).items if op.status == "completed"),
        key=lambda op: op.started_at,
    )


def _pages(client: ApiClient, path: str) -> list[ApiResponse]:
    pages: list[ApiResponse] = []
    cursor: str | None = None
    for _ in range(50):
        params: dict[str, str | int] = {"limit": 100}
        if cursor:
            params["cursor"] = cursor
        page = client.get(path, params=params)
        assert page.status_code == 200, f"{page.status_code} {page.text[:300]}"

        pages.append(page)
        cursor = page.parse(Cursor).next_cursor
        if not cursor:
            return pages

    raise AssertionError(f"{path} never ran out of pages")


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_the_latest_persons_seed_accounts_for_every_account_the_operator_observes(
    admin_operator_session: PersonaSession, stand_manifest: Manifest
) -> None:
    client = admin_operator_session.client
    listed = _latest_completed_seed(client)

    detail = client.get(f"{SEED_JOURNAL}/{listed.operation_id}")
    assert detail.status_code == 200, f"{detail.status_code} {detail.text[:300]}"
    operation = detail.parse(Operation)
    assert operation == listed, f"the detail differs from its listing row:\n{operation}\n{listed}"
    assert operation.operation_type == "persons-seed"
    assert str(operation.insight_tenant_id) == stand_manifest.tenant
    assert operation.completed_at is not None and operation.completed_at >= operation.started_at
    assert operation.summary is not None, "a completed persons-seed carries no summary"

    summary = SeedSummary.model_validate(operation.summary)
    dispositions = (
        summary.accounts_reused_known
        + summary.accounts_linked_by_email
        + summary.accounts_minted_new
        + summary.accounts_minted_from_roster
        + summary.accounts_skipped_closed
        + summary.accounts_skipped_no_email
        + summary.accounts_skipped_no_source_id
        + summary.skipped_contested_email
        + summary.skipped_excluded
    )
    assert summary.accounts_read > 0, f"the seed read no account: {summary}"
    assert dispositions == summary.accounts_read, (
        f"the seed decided {dispositions} accounts out of {summary.accounts_read} it read: {summary}"
    )

    queue = client.get(identity_path("/v1/resolution/attention"))
    assert queue.status_code == 200, f"{queue.status_code} {queue.text[:300]}"
    observed = queue.parse(AttentionResponse).rates.observed
    assert observed == summary.accounts_read, (
        f"the seed read {summary.accounts_read} accounts, the operator observes {observed}"
    )


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_the_seed_linked_the_observed_accounts_to_the_roster_the_manifest_names(
    admin_operator_session: PersonaSession, stand_manifest: Manifest
) -> None:
    client = admin_operator_session.client
    elsewhere = {
        stand_manifest.fixtures[name].uuid
        for name in ("other_tenant_lead",)
        if name in stand_manifest.fixtures
    }
    roster = {person.uuid for person in stand_manifest.personas} - elsewhere
    operator = stand_manifest.fixture("admin_operator").uuid

    persons = {
        str(card.person_id)
        for page in _pages(client, identity_path("/v1/persons"))
        for card in page.parse(PersonListResponse).items
    }
    assert roster <= persons, f"seeded personas missing from the roster: {sorted(roster - persons)}"
    assert not persons & elsewhere, "another tenant's persona is on this tenant's roster"

    holders = {
        str(account.person.person_id)
        for page in _pages(client, identity_path("/v1/resolution/accounts"))
        for account in page.parse(AccountSearchResponse).items
        if account.person is not None and not account.excluded
    }
    assert holders == roster - {operator}, (
        f"observed accounts are held by {len(holders)} persons, the seed's organisation has "
        f"{len(roster - {operator})}: missing {sorted(roster - {operator} - holders)}, "
        f"unexpected {sorted(holders - roster)}"
    )


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_the_persons_sync_journal_reports_rows_copied_and_never_moves_its_watermark_back(
    admin_operator_session: PersonaSession,
) -> None:
    client = admin_operator_session.client
    completed = _completed_syncs(client)
    summaries = [SyncSummary.model_validate(op.summary) for op in completed]
    published = [
        (op, summary)
        for op, summary in zip(completed, summaries, strict=True)
        if summary.outcome == "published"
    ]
    assert published, "a seeded stand has no published persons-sync"

    latest, latest_summary = published[-1]
    detail = client.get(f"{SYNC_JOURNAL}/{latest.operation_id}")
    assert detail.status_code == 200, f"{detail.status_code} {detail.text[:300]}"
    assert detail.parse(PersonsSyncOperationResponse) == latest
    assert latest_summary.rows is not None and latest_summary.rows > 0, latest_summary
    assert latest_summary.max_id is not None, latest_summary
    assert latest_summary.synced_at is not None, latest_summary

    watermark: SyncSummary | None = None
    for op, summary in zip(completed, summaries, strict=True):
        if summary.outcome == "already_current":
            if watermark is not None:
                assert summary.max_id == watermark.max_id, (
                    f"{op.operation_id} found the mirror current at {summary.max_id}, "
                    f"but the last publish before it reached {watermark.max_id}"
                )
            continue

        if watermark is not None:
            assert (summary.max_id or 0) >= (watermark.max_id or 0), (
                f"{op.operation_id} published watermark {summary.max_id} after {watermark.max_id}"
            )
            assert (summary.rows or 0) >= (watermark.rows or 0), (
                f"{op.operation_id} copied {summary.rows} rows of an append-only log "
                f"after an earlier publish copied {watermark.rows}"
            )
        watermark = summary


@pytest.mark.requires_seed("admin_operator", "ceo")
@pytest.mark.security
def test_the_realm_admin_is_refused_the_journal_entries_the_operator_read(
    admin_operator_session: PersonaSession, realm_admin_session: PersonaSession
) -> None:
    seed = _latest_completed_seed(admin_operator_session.client)
    syncs = _completed_syncs(admin_operator_session.client)
    assert syncs, "a seeded stand has no completed persons-sync"

    for path in (
        SEED_JOURNAL,
        f"{SEED_JOURNAL}/{seed.operation_id}",
        SYNC_JOURNAL,
        f"{SYNC_JOURNAL}/{syncs[-1].operation_id}",
    ):
        response = realm_admin_session.client.get(path)
        assert response.status_code == 403, f"{path}: {response.status_code} {response.text[:300]}"
        assert response.content_type.startswith(PROBLEM_CONTENT_TYPE), response.content_type
        assert response.parse(ProblemDocument).status == 403
