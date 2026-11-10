from __future__ import annotations

import pytest
from insight_stand import (
    SCRATCH_SOURCE_ID,
    SCRATCH_SOURCE_TYPE,
    ApiClient,
    Manifest,
    PersonaSession,
    identity_path,
)

from ..schemas import (
    PROBLEM_CONTENT_TYPE,
    AccountBindingResponse,
    AccountSearchResponse,
    AttentionResponse,
    PersonAccountsResponse,
    PersonListResponse,
    ProblemDocument,
    SourcesResponse,
)
from ..schemas.identity import AccountMatchResponse, PersonAccountEntry

PERSONS = identity_path("/v1/persons")
ACCOUNT_SEARCH = identity_path("/v1/resolution/accounts")
SOURCES = identity_path("/v1/resolution/sources")
ATTENTION = identity_path("/v1/resolution/attention")


def _accounts_of(client: ApiClient, person_id: str) -> PersonAccountsResponse:
    response = client.get(identity_path(f"/v1/resolution/persons/{person_id}/accounts"))
    assert response.status_code == 200, f"{response.status_code} {response.text[:300]}"
    return response.parse(PersonAccountsResponse)


def _seeded(accounts: list[PersonAccountEntry]) -> list[PersonAccountEntry]:
    return [
        account
        for account in accounts
        if not (
            account.source == SCRATCH_SOURCE_TYPE and str(account.source_id) == SCRATCH_SOURCE_ID
        )
    ]


def _binding_path(account: PersonAccountEntry) -> str:
    return identity_path(
        f"/v1/resolution/accounts/{account.source}/{account.source_id}/{account.account_id}"
    )


def _every_observed_account(client: ApiClient) -> list[AccountMatchResponse]:
    found: list[AccountMatchResponse] = []
    cursor: str | None = None
    for _ in range(50):
        params: dict[str, str | int] = {"limit": 100}
        if cursor:
            params["cursor"] = cursor
        page = client.get(ACCOUNT_SEARCH, params=params)
        assert page.status_code == 200, f"{page.status_code} {page.text[:300]}"

        listing = page.parse(AccountSearchResponse)
        found.extend(listing.items)
        cursor = listing.next_cursor
        if not cursor:
            return found

    raise AssertionError("the account listing never ran out of pages")


@pytest.mark.requires_seed("admin_operator", "dev_lead")
@pytest.mark.reliability
def test_the_operator_finds_the_lead_by_an_email_fragment_and_every_account_behind_them(
    admin_operator_session: PersonaSession, stand_manifest: Manifest
) -> None:
    client = admin_operator_session.client
    lead = stand_manifest.fixture("dev_lead")
    fragment = lead.email.split("@")[0]

    search = client.get(PERSONS, params={"q": fragment})
    assert search.status_code == 200, f"{search.status_code} {search.text[:300]}"
    cards = [card for card in search.parse(PersonListResponse).items if card.email == lead.email]
    assert [str(card.person_id) for card in cards] == [lead.uuid], (
        f"searching {fragment!r} did not find exactly {lead.email}: {cards}"
    )
    person_id = str(cards[0].person_id)

    owned = _accounts_of(client, person_id)
    assert str(owned.person_id) == person_id
    accounts = _seeded(owned.accounts)
    assert accounts, f"{lead.email} holds no seeded account"

    sources = client.get(SOURCES)
    assert sources.status_code == 200, f"{sources.status_code} {sources.text[:300]}"
    reported = set(sources.parse(SourcesResponse).items)

    idp = stand_manifest.capabilities.idp
    logins = [account for account in accounts if account.source == idp]
    assert [login.account_id for login in logins] == [lead.uuid], (
        f"the lead's {idp} login is not the account their sign-in subject names: {logins}"
    )
    connector_accounts = [account for account in accounts if account.source != idp]
    assert connector_accounts, f"{lead.email} holds no account any connector reported"

    for account in accounts:
        read = client.get(_binding_path(account))
        assert read.status_code == 200, f"{account}: {read.status_code} {read.text[:300]}"
        binding = read.parse(AccountBindingResponse)

        assert (binding.source, binding.source_id, binding.account_id) == (
            account.source,
            account.source_id,
            account.account_id,
        )
        assert str(binding.person_id) == person_id, (
            f"{account.source}:{account.account_id} lists under {lead.email} but its "
            f"binding names {binding.person_id}"
        )
        assert binding.history, f"{account.source}:{account.account_id} has no decision on record"
        assert str(binding.history[0].person_id) == person_id
        assert binding.history[0].by_operator is account.bound_by_operator, (
            f"the list and the account's own history disagree on who decided "
            f"{account.source}:{account.account_id}"
        )

    for account in connector_accounts:
        assert account.source in reported, (
            f"{account.source} reported {lead.email}'s account but is not a listed source: "
            f"{sorted(reported)}"
        )

        narrowed = client.get(
            ACCOUNT_SEARCH,
            params={"source": account.source, "q": account.account_id, "limit": 100},
        )
        assert narrowed.status_code == 200, f"{narrowed.status_code} {narrowed.text[:300]}"
        matches = [
            match
            for match in narrowed.parse(AccountSearchResponse).items
            if (match.source_id, match.account_id) == (account.source_id, account.account_id)
        ]
        assert len(matches) == 1, (
            f"filtering by {account.source} does not list {account.account_id}: {matches}"
        )
        assert matches[0].person is not None, f"{account.account_id} lists with no holder"
        assert str(matches[0].person.person_id) == person_id
        assert matches[0].excluded is False

    queue = client.get(ATTENTION)
    assert queue.status_code == 200, f"{queue.status_code} {queue.text[:300]}"
    held = {(account.source, account.source_id, account.account_id) for account in accounts}
    waiting = [
        item
        for item in queue.parse(AttentionResponse).items
        if (item.source, item.source_id, item.account_id) in held
    ]
    assert not waiting, f"accounts already bound to {lead.email} still await a decision: {waiting}"


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_the_account_listing_and_the_queue_rates_count_the_same_accounts(
    admin_operator_session: PersonaSession,
) -> None:
    client = admin_operator_session.client

    queue = client.get(ATTENTION)
    assert queue.status_code == 200, f"{queue.status_code} {queue.text[:300]}"
    rates = queue.parse(AttentionResponse).rates

    listed = _every_observed_account(client)
    keys = [(item.source, item.source_id, item.account_id) for item in listed]
    assert len(keys) == len(set(keys)), "an account is listed twice"

    excluded = [item for item in listed if item.excluded]
    bound = [item for item in listed if item.person is not None and not item.excluded]
    unbound = [item for item in listed if item.person is None and not item.excluded]

    assert len(listed) == rates.observed, (
        f"the listing walks {len(listed)} accounts, the rates observe {rates.observed}"
    )
    assert len(bound) == rates.bound, f"{len(bound)} listed with a holder, rates say {rates}"
    assert len(excluded) == rates.excluded, f"{len(excluded)} listed excluded, rates say {rates}"
    assert len(unbound) == rates.pending + rates.no_source_id + rates.no_evidence, (
        f"{len(unbound)} listed with no holder, rates say {rates}"
    )


@pytest.mark.requires_seed("admin_operator", "ceo", "dev_lead")
@pytest.mark.security
def test_the_realm_admin_is_refused_the_accounts_the_operator_just_read(
    admin_operator_session: PersonaSession,
    realm_admin_session: PersonaSession,
    stand_manifest: Manifest,
) -> None:
    lead = stand_manifest.fixture("dev_lead")
    accounts = _seeded(_accounts_of(admin_operator_session.client, lead.uuid).accounts)
    assert accounts, f"{lead.email} holds no seeded account"

    for path in (
        identity_path(f"/v1/resolution/persons/{lead.uuid}/accounts"),
        _binding_path(accounts[0]),
    ):
        response = realm_admin_session.client.get(path)
        assert response.status_code == 403, f"{path}: {response.status_code} {response.text[:300]}"
        assert response.content_type.startswith(PROBLEM_CONTENT_TYPE), response.content_type
        assert response.parse(ProblemDocument).status == 403
