"""The operator correction surface — the manual-resolution routes on the deployed path.

    GET  /v1/resolution/attention                         200 rates arithmetic · 403 realm admin
    GET  /v1/resolution/accounts/{source}/{sid}/{aid}     200 binding + history (round trip)
    GET  /v1/resolution/persons/{id}/accounts             200 for a seeded person
    POST /v1/resolution/bind                              200 applied → already_decided (round trip) · 400 excluded sentinel
    POST /v1/resolution/merge                             400 source == target · excluded sentinel (validation, no write)
    POST /v1/resolution/detach                            404 unseen account (no write)
    POST /v1/resolution/exclude                           200 applied (the round trip's cleanup)

The verbs append to an append-only journal, so the scratch policy cannot be
"create and delete". The round trip instead ends in `exclude`, whose end state
is invisible to every read surface by contract: the account leaves the
person's account list, never enters the review queue (it carries no connector
evidence), and resolves as no person. What persists is journal rows — the same
thing every operator decision leaves behind by design. The account id carries
`scratch_name`'s run tag so a human reading the journal can attribute it.

Merge and detach are exercised through pure-validation refusals: proving the
route answers with a session without moving a single seeded binding — the
stand must read the same before and after this module.

The 401 half is in `test_gateway.py`, swept over every operation.
"""

from __future__ import annotations

import pytest
from insight_stand import ApiClient, Manifest, PersonaSession, identity_path

from .. import scratch
from ..schemas import (
    AccountBindingResponse,
    AttentionResponse,
    CorrectionResponse,
    PersonAccountsResponse,
)

ATTENTION = identity_path("/v1/resolution/attention")

#: The reserved excluded-person sentinel (`Uuid::from_u128(u128::MAX)` in the
#: service). Once any account was ever excluded it exists in the journal, so
#: only an explicit guard keeps it out of bind/merge — these tests pin that
#: guard.
EXCLUDED_PERSON = "ffffffff-ffff-ffff-ffff-ffffffffffff"

#: A connector-instance id for accounts that exist only in this module. Fixed,
#: not random: the coverage template folds `{id}` per path segment either way,
#: and a stable value keeps journal rows attributable to this suite.
SCRATCH_SOURCE_ID = "01900000-0000-7000-8000-00000000feed"


def _account_path(account_id: str) -> str:
    return identity_path(
        f"/v1/resolution/accounts/github/{SCRATCH_SOURCE_ID}/{account_id}"
    )


@pytest.mark.security
def test_an_unauthenticated_caller_never_reaches_any_of_this(api_client: ApiClient) -> None:
    """Proven per operation by `test_gateway.py`; spot-checked here so this
    module carries its own reason for using a session at all."""
    response = api_client.get(ATTENTION)
    assert response.status_code == 401, f"{response.status_code} {response.text[:300]}"


@pytest.mark.requires_seed("ceo")
@pytest.mark.security
def test_the_realm_admin_is_refused_the_queue(realm_admin_session: PersonaSession) -> None:
    """The correction surface reads `person_roles`, never the realm role —
    the same boundary `test_admin.py` pins for the CRUD listings."""
    response = realm_admin_session.client.get(ATTENTION)
    assert response.status_code == 403, f"{response.status_code} {response.text[:300]}"


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_the_queue_answers_with_coherent_tenant_wide_rates(
    admin_operator_session: PersonaSession,
) -> None:
    """`rates` counts every observed account, and the states partition it:
    an account is bound, pending, evidence-less or excluded — never two at
    once and never none. A sum mismatch means the fold dropped or
    double-counted accounts, which the UI would show as a lying match rate.
    """
    response = admin_operator_session.client.get(ATTENTION)
    assert response.status_code == 200, f"{response.status_code} {response.text[:300]}"

    queue = response.parse(AttentionResponse)
    rates = queue.rates
    # The floor first: 0 == 0+0+0+0 partitions perfectly, so without it a
    # fold that silently reads zero rows (a broken join, a missing relation
    # mapped to an empty answer) would pass the arithmetic below. The seed
    # this test requires materializes identity evidence for the roster, so a
    # seeded stand always has observed accounts.
    assert rates.observed > 0, f"a seeded stand reports zero observed accounts: {rates}"
    assert rates.observed == rates.bound + rates.pending + rates.no_evidence + rates.excluded, (
        f"resolution states do not partition the observed set: {rates}"
    )
    assert len(queue.items) <= rates.pending + rates.no_evidence, (
        "more queue items than undecided accounts"
    )
    assert queue.truncated is False, (
        "the seeded roster is far below the evidence cap — a truncated answer "
        "means the read is broken, not that the stand is big"
    )
    # Two distinct caps, and the seeded roster clears both: a set flag here
    # would mean the queue is a prefix, which makes the assertion above about
    # its length meaningless.
    assert queue.items_truncated is False, (
        "the queue reports its item cap was hit on a roster far below it"
    )


@pytest.mark.requires_seed("admin_operator", "dev_lead")
@pytest.mark.reliability
def test_a_seeded_person_lists_their_accounts(
    admin_operator_session: PersonaSession, stand_manifest: Manifest
) -> None:
    """The route echoes the requested id, so the id check alone proves
    nothing; the list itself must be non-empty. Every roster persona gets a
    login-bootstrap identity row from the seed, so a seeded person with zero
    accounts means the query read the wrong place, not an empty stand."""
    lead = stand_manifest.fixture("dev_lead")
    response = admin_operator_session.client.get(
        identity_path(f"/v1/resolution/persons/{lead.uuid}/accounts")
    )
    assert response.status_code == 200, f"{response.status_code} {response.text[:300]}"

    owned = response.parse(PersonAccountsResponse)
    assert str(owned.person_id) == lead.uuid
    assert owned.accounts, (
        f"a seeded person lists no accounts — the seed guarantees {lead.email} "
        "at least their login-bootstrap identity row"
    )


@pytest.mark.requires_seed("admin_operator", "dev_lead")
@pytest.mark.reliability
def test_bind_confirm_and_exclude_round_trip(
    admin_operator_session: PersonaSession, stand_manifest: Manifest
) -> None:
    """One synthetic account through the operator's grammar.

    bind → `applied`; the same bind again → `already_decided` (an operator
    re-asserting their own decision is idempotent, not an error); the account
    read shows the person and an operator-authored history entry; exclude →
    `applied`, after which the account is off the person's list — the
    invisible end state that stands in for deletion on an append-only journal.
    """
    client = admin_operator_session.client
    lead = stand_manifest.fixture("dev_lead")
    account_id = scratch.scratch_name("resolution")
    account = {"source": "github", "source_id": SCRATCH_SOURCE_ID, "id": account_id}

    bind = client.post(
        identity_path("/v1/resolution/bind"),
        json_body={"bindings": [{"account": account, "person_id": lead.uuid}]},
    )
    assert bind.status_code == 200, f"bind: {bind.status_code} {bind.text[:300]}"
    first = bind.parse(CorrectionResponse)
    assert first.applied == 1 and first.items[0].outcome == "applied", first

    # From here the scratch account is bound to a REAL seeded persona, and no
    # sweep can see a leaked binding (the journal has no scratch listing) —
    # so the exclude must run even when an assertion in between fails.
    try:
        again = client.post(
            identity_path("/v1/resolution/bind"),
            json_body={"bindings": [{"account": account, "person_id": lead.uuid}]},
        )
        assert again.status_code == 200, f"rebind: {again.status_code} {again.text[:300]}"
        second = again.parse(CorrectionResponse)
        assert second.already_decided == 1 and second.items[0].outcome == "already_decided", second

        read = client.get(_account_path(account_id))
        assert read.status_code == 200, f"read: {read.status_code} {read.text[:300]}"
        binding = read.parse(AccountBindingResponse)
        assert str(binding.person_id) == lead.uuid
        assert any(entry.by_operator for entry in binding.history), (
            f"no operator-authored entry in {binding.history}"
        )
    finally:
        exclude = client.post(
            identity_path("/v1/resolution/exclude"),
            json_body={"account": account, "comment": "stand scratch cleanup"},
        )
        assert exclude.status_code == 200, f"exclude: {exclude.status_code} {exclude.text[:300]}"
        assert exclude.parse(CorrectionResponse).applied == 1

    owned = client.get(
        identity_path(f"/v1/resolution/persons/{lead.uuid}/accounts")
    ).parse(PersonAccountsResponse)
    assert account_id not in [a.account_id for a in owned.accounts], (
        "the excluded scratch account still lists under the person — the stand is dirty"
    )


@pytest.mark.requires_seed("admin_operator", "dev_lead")
@pytest.mark.reliability
def test_merge_refuses_a_person_merged_into_themselves(
    admin_operator_session: PersonaSession, stand_manifest: Manifest
) -> None:
    """Pure validation — proves the route answers a session without moving a
    single seeded binding."""
    lead = stand_manifest.fixture("dev_lead")
    response = admin_operator_session.client.post(
        identity_path("/v1/resolution/merge"),
        json_body={"source_person_id": lead.uuid, "target_person_id": lead.uuid},
    )
    assert response.status_code == 400, f"{response.status_code} {response.text[:300]}"


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.security
def test_the_excluded_sentinel_is_not_a_bind_target(
    admin_operator_session: PersonaSession,
) -> None:
    """Binding to the sentinel would be an exclude that skips the
    known-account check (bind deliberately pre-registers unseen accounts).
    Pure validation: refused before any existence lookup, so no write."""
    response = admin_operator_session.client.post(
        identity_path("/v1/resolution/bind"),
        json_body={
            "bindings": [
                {
                    "account": {
                        "source": "github",
                        "source_id": SCRATCH_SOURCE_ID,
                        "id": "never-observed-account",
                    },
                    "person_id": EXCLUDED_PERSON,
                }
            ]
        },
    )
    assert response.status_code == 400, f"{response.status_code} {response.text[:300]}"


@pytest.mark.requires_seed("admin_operator", "dev_lead")
@pytest.mark.security
@pytest.mark.parametrize("side", ["source_person_id", "target_person_id"])
def test_the_excluded_sentinel_is_not_a_merge_side(
    admin_operator_session: PersonaSession, stand_manifest: Manifest, side: str
) -> None:
    """A merge naming the sentinel moves EVERY excluded account of the tenant
    at once (as source), or mass-excludes a person's accounts while the
    journal records a merge (as target). Refused as validation, no write."""
    lead = stand_manifest.fixture("dev_lead")
    body = {"source_person_id": lead.uuid, "target_person_id": lead.uuid}
    body[side] = EXCLUDED_PERSON
    response = admin_operator_session.client.post(
        identity_path("/v1/resolution/merge"), json_body=body
    )
    assert response.status_code == 400, f"{response.status_code} {response.text[:300]}"


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_detach_refuses_an_account_nobody_ever_saw(
    admin_operator_session: PersonaSession,
) -> None:
    """Binding an unseen account is allowed (pre-registration); detaching one
    is not — there is nothing to detach. 404, and no write happens."""
    response = admin_operator_session.client.post(
        identity_path("/v1/resolution/detach"),
        json_body={
            "account": {
                "source": "github",
                "source_id": SCRATCH_SOURCE_ID,
                "id": "never-observed-account",
            }
        },
    )
    assert response.status_code == 404, f"{response.status_code} {response.text[:300]}"
