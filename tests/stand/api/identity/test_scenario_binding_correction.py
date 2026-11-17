from __future__ import annotations

import pytest
from insight_stand import (
    SCRATCH_SOURCE_ID,
    SCRATCH_SOURCE_TYPE,
    ApiClient,
    JsonValue,
    Manifest,
    PersonaSession,
    identity_path,
    scratch_name,
)

from ..schemas import (
    AccountBindingResponse,
    AttentionResponse,
    CorrectionResponse,
    PersonAccountsResponse,
)
from ..schemas.identity import ResolutionRatesResponse

BIND = identity_path("/v1/resolution/bind")
EXCLUDE = identity_path("/v1/resolution/exclude")
ATTENTION = identity_path("/v1/resolution/attention")
EXCLUDED_PERSON = "ffffffff-ffff-ffff-ffff-ffffffffffff"


def _account(account_id: str) -> dict[str, JsonValue]:
    return {"source": SCRATCH_SOURCE_TYPE, "source_id": SCRATCH_SOURCE_ID, "id": account_id}


def _binding(client: ApiClient, account_id: str) -> AccountBindingResponse:
    response = client.get(
        identity_path(
            f"/v1/resolution/accounts/{SCRATCH_SOURCE_TYPE}/{SCRATCH_SOURCE_ID}/{account_id}"
        )
    )
    assert response.status_code == 200, f"{response.status_code} {response.text[:300]}"
    return response.parse(AccountBindingResponse)


def _listed_under(client: ApiClient, person_id: str) -> dict[str, bool]:
    response = client.get(identity_path(f"/v1/resolution/persons/{person_id}/accounts"))
    assert response.status_code == 200, f"{response.status_code} {response.text[:300]}"
    return {
        account.account_id: account.bound_by_operator
        for account in response.parse(PersonAccountsResponse).accounts
        if account.source == SCRATCH_SOURCE_TYPE and str(account.source_id) == SCRATCH_SOURCE_ID
    }


def _rates(client: ApiClient) -> ResolutionRatesResponse:
    response = client.get(ATTENTION)
    assert response.status_code == 200, f"{response.status_code} {response.text[:300]}"
    return response.parse(AttentionResponse).rates


def _bind(client: ApiClient, pairs: list[tuple[str, str]], comment: str) -> CorrectionResponse:
    response = client.post(
        BIND,
        json_body={
            "bindings": [
                {"account": _account(account_id), "person_id": person_id}
                for account_id, person_id in pairs
            ],
            "comment": comment,
        },
    )
    assert response.status_code == 200, f"bind: {response.status_code} {response.text[:300]}"
    return response.parse(CorrectionResponse)


def _exclude_all(client: ApiClient, account_ids: list[str]) -> None:
    failures = []
    for account_id in account_ids:
        response = client.post(
            EXCLUDE, json_body={"account": _account(account_id), "comment": "stand scratch cleanup"}
        )
        if response.status_code == 404:
            continue
        if response.status_code != 200:
            failures.append(f"{account_id}: {response.status_code} {response.text[:200]}")
            continue
        outcome = response.parse(CorrectionResponse).items[0].outcome
        if outcome not in {"applied", "already_decided"}:
            failures.append(f"{account_id}: exclude {outcome}")

    assert not failures, "scratch accounts left bound to seeded persons:\n" + "\n".join(failures)


@pytest.mark.requires_seed("admin_operator", "dev_lead", "development_ic")
@pytest.mark.reliability
def test_an_operator_rebinding_a_synthetic_account_moves_it_on_every_read(
    admin_operator_session: PersonaSession, stand_manifest: Manifest
) -> None:
    client = admin_operator_session.client
    operator = admin_operator_session.person.uuid
    lead = stand_manifest.fixture("dev_lead").uuid
    member = stand_manifest.fixture("development_ic").uuid
    moved = scratch_name("rebind")
    stayed = scratch_name("rebind")
    rates_before = _rates(client)

    try:
        first = _bind(client, [(moved, lead), (stayed, member)], "stand scenario: first decision")
        assert first.applied == 2, first
        assert [(item.account_id, item.outcome) for item in first.items] == [
            (moved, "applied"),
            (stayed, "applied"),
        ]

        assert _listed_under(client, lead).get(moved) is True, (
            f"{moved} is not listed under the lead as an operator decision"
        )
        assert stayed not in _listed_under(client, lead)
        assert _listed_under(client, member).get(stayed) is True
        assert moved not in _listed_under(client, member)

        bound = _binding(client, moved)
        assert str(bound.person_id) == lead
        assert str(bound.history[0].person_id) == lead
        assert str(bound.history[0].author_person_id) == operator
        assert bound.history[0].author is not None
        assert str(bound.history[0].author.person_id) == operator
        assert bound.history[0].by_operator is True
        decision = bound.operations[0]
        assert (decision.verb, decision.comment, decision.accounts_touched, decision.outcome) == (
            "operator-bind",
            "stand scenario: first decision",
            2,
            "applied",
        )
        assert str(decision.author_person_id) == operator

        second = _bind(client, [(moved, member)], "stand scenario: second decision")
        assert second.applied == 1 and second.items[0].outcome == "applied", second

        assert moved not in _listed_under(client, lead), (
            f"{moved} still lists under the lead after the operator moved it"
        )
        assert set(_listed_under(client, member)) >= {moved, stayed}

        rebound = _binding(client, moved)
        assert str(rebound.person_id) == member
        assert [str(entry.person_id) for entry in rebound.history] == [member, lead], (
            f"the history does not name both decisions newest first: {rebound.history}"
        )
        assert [(op.comment, op.accounts_touched) for op in rebound.operations] == [
            ("stand scenario: second decision", 1),
            ("stand scenario: first decision", 2),
        ]

        assert _rates(client) == rates_before, (
            "binding accounts no connector reported moved the tenant's resolution rates"
        )
    finally:
        _exclude_all(client, [moved, stayed])

    for person in (lead, member):
        assert not {moved, stayed} & set(_listed_under(client, person)), (
            f"an excluded scratch account still lists under {person}"
        )

    excluded = _binding(client, moved)
    assert str(excluded.person_id) == EXCLUDED_PERSON
    assert excluded.history[0].person is None
    assert excluded.operations[0].verb == "operator-exclude"
    assert [str(entry.person_id) for entry in excluded.history[1:]] == [member, lead]
