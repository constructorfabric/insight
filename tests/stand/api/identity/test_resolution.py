"""The operator correction surface — the manual-resolution routes on the deployed path.

    GET  /v1/resolution/attention                         200 rates arithmetic · 403 realm admin
    GET  /v1/resolution/accounts                          200 search + holder + by source · 400 stale cursor · 400 needle over cap · 403 realm admin
    GET  /v1/resolution/accounts/{source}/{sid}/{aid}     200 binding + history (round trip)
    GET  /v1/resolution/persons/{id}/accounts             200 for a seeded person
    POST /v1/resolution/bind                              200 applied → already_decided (round trip) · 400 excluded sentinel · 400 empty / over cap / named twice
    POST /v1/resolution/merge                             400 source == target · excluded sentinel · 404 unknown person, either side (validation, no write)
    POST /v1/resolution/detach                            404 unseen account (no write)
    POST /v1/resolution/exclude                           200 applied (the round trip's cleanup) · 404 unseen account
    POST /v1/resolution/{bind,merge,detach,exclude}       400 unparseable body · 400 comment over cap · 415 wrong media type

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

Every refusal here is asserted through `_refused`, which reads the problem
document rather than the status line alone: the console renders
`field_violations` as "which box you got wrong", so a refusal that degraded
to an unreadable body would leave the operator staring at generic failure
text with the status assertion still green.

The 401 half is in `test_gateway.py`, swept over every operation.
"""

from __future__ import annotations

import json

import pytest
from insight_stand import (
    ApiClient,
    ApiResponse,
    JsonValue,
    Manifest,
    PersonaSession,
    identity_path,
)

from .. import scratch
from ..schemas import (
    PROBLEM_CONTENT_TYPE,
    AccountBindingResponse,
    AccountSearchResponse,
    AttentionResponse,
    CorrectionResponse,
    PersonAccountsResponse,
    ProblemDocument,
)
from ..scratch import SCRATCH_SOURCE_ID, SCRATCH_SOURCE_TYPE

#: The correction verbs, each with a body its extractor accepts, so a case
#: that varies one thing draws a refusal about that thing. The ids name
#: nobody — a refusal at the extractor or a validator must not depend on the
#: referenced rows existing.
CORRECTION_VERBS: tuple[tuple[str, str], ...] = (
    ("bind", "bindings"),
    ("merge", "source_person_id"),
    ("detach", "account"),
    ("exclude", "account"),
)

ATTENTION = identity_path("/v1/resolution/attention")
ACCOUNT_SEARCH = identity_path("/v1/resolution/accounts")

#: The reserved excluded-person sentinel (`Uuid::from_u128(u128::MAX)` in the
#: service). Once any account was ever excluded it exists in the journal, so
#: only an explicit guard keeps it out of bind/merge — these tests pin that
#: guard.
EXCLUDED_PERSON = "ffffffff-ffff-ffff-ffff-ffffffffffff"

#: Well-formed, right type, names nobody — so the refusal it draws is the
#: existence check rather than the uuid parser.
UNKNOWN_PERSON = "00000000-0000-0000-0000-000000000000"


def _account(account_id: str) -> dict[str, JsonValue]:
    """One account under the suite's own connector instance — the only place a
    correction this module writes may land (`scratch.py` rule 5)."""
    return {"source": SCRATCH_SOURCE_TYPE, "source_id": SCRATCH_SOURCE_ID, "id": account_id}


def _account_path(account_id: str) -> str:
    return identity_path(
        f"/v1/resolution/accounts/{SCRATCH_SOURCE_TYPE}/{SCRATCH_SOURCE_ID}/{account_id}"
    )


def _refused(
    response: ApiResponse,
    code: int,
    field: str | None = None,
    resource: str | None = None,
) -> None:
    """A correction refusal is only actionable if the operator can read it.

    A validation refusal is built by `invalid()`, which stamps
    `field_violations` into the problem document — what the console turns into
    "which box you got wrong". A 404 instead names the thing that was not
    found in `resource_name`, and that field is what separates two refusals
    that read alike: both merge sides answer "person not found", so a merge
    checking only its source would pass the target case on the source's
    refusal. Asserting the status alone would keep passing through either
    degradation, with the operator left on generic failure text.
    """
    assert response.status_code == code, f"{response.status_code} {response.text[:300]}"
    # The media type is half the contract: a body that parses as a problem
    # document but arrives as `application/json` is not one, and a consumer
    # switching on content type would fall through to its generic handler.
    assert response.content_type.startswith(PROBLEM_CONTENT_TYPE), (
        f"refusal arrived as {response.content_type!r}, not {PROBLEM_CONTENT_TYPE!r}"
    )

    problem = response.parse(ProblemDocument)
    assert problem.status == code, problem
    if field is not None:
        violations = problem.context.get("field_violations")
        assert isinstance(violations, list) and violations, (
            f"no field_violations to act on: {problem.context}"
        )
        named = [
            entry.get("field") for entry in violations if isinstance(entry, dict)
        ]
        assert field in named, (
            f"refusal names {named}, not the field the caller got wrong ({field})"
        )

    if resource is not None:
        assert problem.context.get("resource_name") == resource, (
            f"the refusal names {problem.context.get('resource_name')!r} rather than the "
            f"thing that was not found ({resource!r})"
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


@pytest.mark.requires_seed("ceo")
@pytest.mark.security
def test_the_realm_admin_is_refused_the_account_search(
    realm_admin_session: PersonaSession,
) -> None:
    """The gate runs before `q` validation: a non-admin with no query at all
    still learns nothing but 403 — a 400 would reveal the gate ran second."""
    response = realm_admin_session.client.get(ACCOUNT_SEARCH)
    assert response.status_code == 403, f"{response.status_code} {response.text[:300]}"


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_no_needle_lists_the_observed_accounts_a_page_at_a_time(
    admin_operator_session: PersonaSession,
) -> None:
    """A short or absent needle used to be refused because it would answer with
    everything. It still answers with everything — one bounded page of it — so
    an operator can read what the connectors reported instead of guessing a
    value to search for."""
    for params in (None, {"q": "ab"}):
        response = admin_operator_session.client.get(ACCOUNT_SEARCH, params=params)
        assert response.status_code == 200, (
            f"q={params!r} answered {response.status_code}: {response.text[:300]}"
        )
        listing = response.parse(AccountSearchResponse)
        # An empty list would satisfy the bound too, and the service answers one
        # for a fold it cannot read — so a query aimed at the wrong relation, or
        # a stand whose identity build never ran, would read as a pass here.
        assert listing.items, f"q={params!r} listed no observed account"
        assert len(listing.items) <= 20, "the default page is the bound"


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_a_connector_name_lists_the_accounts_it_reported(
    admin_operator_session: PersonaSession,
) -> None:
    """The row prints the source beside the account id, so typing a connector's
    name is a question the list already looks like it answers. The name comes
    from what this stand actually reports rather than a constant: a source that
    is not installed here would make the assertion vacuous.
    """
    seen = admin_operator_session.client.get(ACCOUNT_SEARCH, params={"limit": 100})
    assert seen.status_code == 200, f"{seen.status_code} {seen.text[:300]}"
    sources = sorted({item.source for item in seen.parse(AccountSearchResponse).items})
    assert sources, "a seeded stand reports no observed accounts"
    source = sources[0]

    response = admin_operator_session.client.get(ACCOUNT_SEARCH, params={"q": source})
    assert response.status_code == 200, f"{response.status_code} {response.text[:300]}"

    found = response.parse(AccountSearchResponse)
    assert found.items, f"searching {source!r} lists none of that connector's accounts"
    assert any(match.source == source for match in found.items), (
        f"no match actually comes from {source!r}: {found.items[:3]}"
    )
    # A connector name can also occur inside an address or a handle, so the rule
    # is the needle reaching SOME searched value — not the source alone. The one
    # searched value this response cannot show is the name composed from first +
    # last, so a match reachable only through that would fail here despite the
    # server behaving: read a failure as "check which value carried it" before
    # reading it as a filter defect.
    lowered = source.lower()
    for match in found.items:
        carried = [
            value
            for value in (
                match.source,
                match.email,
                match.username,
                match.display_name,
                match.account_id,
            )
            if value is not None
        ]
        assert any(lowered in value.lower() for value in carried), (
            f"match carries the needle in no searched value: {match!r}"
        )


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_the_account_listing_pages_without_repeating_an_account(
    admin_operator_session: PersonaSession,
) -> None:
    """The fold has no order of its own, so paging over it is only sound
    because the listing imposes one. A repeat here would mean an operator sees
    the same account twice and a skip would hide one entirely."""
    seen: list[str] = []
    cursor: str | None = None
    for _ in range(4):
        params: dict[str, object] = {"limit": 1}
        if cursor:
            params["cursor"] = cursor
        page = admin_operator_session.client.get(ACCOUNT_SEARCH, params=params)
        assert page.status_code == 200, f"{page.status_code} {page.text[:300]}"
        listing = page.parse(AccountSearchResponse)
        seen.extend(f"{item.source}:{item.source_id}:{item.account_id}" for item in listing.items)
        cursor = listing.next_cursor
        if not cursor:
            break

    # A walk that stops on page one proves nothing about a boundary, and one
    # account never repeats itself.
    assert len(seen) > 1, "the walk never left the first page — no cursor was issued"
    assert len(seen) == len(set(seen)), f"an account appeared on two pages: {seen}"

    # Uniqueness alone cannot see a skip: a cursor that jumps forward still
    # returns distinct accounts. Retracing against one whole page can.
    whole_page = admin_operator_session.client.get(ACCOUNT_SEARCH, params={"limit": len(seen)})
    listed = [
        f"{item.source}:{item.source_id}:{item.account_id}"
        for item in whole_page.parse(AccountSearchResponse).items
    ]
    assert seen == listed, (
        "walking one row at a time must retrace the same accounts in the same order"
    )


@pytest.mark.requires_seed("admin_operator", "dev_lead")
@pytest.mark.reliability
def test_account_search_finds_a_seeded_account_and_names_its_holder(
    admin_operator_session: PersonaSession, stand_manifest: Manifest
) -> None:
    """`GET /v1/resolution/accounts?q=` answers with the account AND whose it
    is — the mode exists for an operator holding a value, not a person. The
    seeded lead's address is observed by the roster connector and bound by the
    seed, so searching it must return at least one account holding a hydrated
    person; every match must echo the needle in one of its searched values.
    """
    lead = stand_manifest.fixture("dev_lead")
    needle = lead.email

    response = admin_operator_session.client.get(ACCOUNT_SEARCH, params={"q": needle})
    assert response.status_code == 200, f"{response.status_code} {response.text[:300]}"

    found = response.parse(AccountSearchResponse)
    assert found.items, f"a seeded stand finds no account for {needle!r}"
    lowered = needle.lower()
    for match in found.items:
        carried = [
            value
            for value in (
                match.source,
                match.email,
                match.username,
                match.display_name,
                match.account_id,
            )
            if value is not None
        ]
        assert any(lowered in value.lower() for value in carried), (
            f"match carries none of the searched values: {match!r}"
        )
    bound = [match for match in found.items if match.person is not None]
    assert bound, "the seeded address resolves to a bound account, so a holder must appear"
    assert any((match.person.email or "").lower() == lowered for match in bound), (
        f"no holder card carries the searched address: {bound!r}"
    )


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_the_queue_answers_with_coherent_tenant_wide_rates(
    admin_operator_session: PersonaSession,
) -> None:
    """`rates` counts every observed account, and the states partition it: an
    account is bound, pending, unbindable without an operator, evidence-less or
    excluded — never two at once and never none. A sum mismatch means the fold
    dropped or double-counted accounts, which the UI would show as a lying
    match rate.
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
    assert (
        rates.observed
        == rates.bound + rates.pending + rates.no_source_id + rates.no_evidence + rates.excluded
    ), f"resolution states do not partition the observed set: {rates}"
    # Over distinct ACCOUNTS, and only the unbound ones. Two reasons the naive
    # count is not an invariant: an item can be a bound account still awaiting a
    # human (an automatic mint, which `rates` counts under `bound`), and one
    # unbound account can hold TWO items — the conflict pass lists every account
    # of a divergent e-mail group, including the unbound member the main pass
    # already reported as contested. `rates` counts each account once, so the
    # comparison only means anything per account.
    unbound_accounts = {
        (item.source, item.source_id, item.account_id)
        for item in queue.items
        if item.bound_to is None
    }
    assert len(unbound_accounts) <= rates.pending + rates.no_source_id + rates.no_evidence, (
        "more unbound accounts in the queue than undecided accounts"
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
    account = _account(account_id)

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

    owned = client.get(identity_path(f"/v1/resolution/persons/{lead.uuid}/accounts")).parse(
        PersonAccountsResponse
    )
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
    _refused(response, 400, field="target_person_id")


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
                    "account": _account("never-observed-account"),
                    "person_id": EXCLUDED_PERSON,
                }
            ]
        },
    )
    _refused(response, 400, field="person_id")


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
    body: dict[str, JsonValue] = {
        "source_person_id": lead.uuid,
        "target_person_id": lead.uuid,
    }
    body[side] = EXCLUDED_PERSON
    response = admin_operator_session.client.post(
        identity_path("/v1/resolution/merge"), json_body=body
    )
    _refused(response, 400, field=side)


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
@pytest.mark.parametrize("verb", ["detach", "exclude"])
def test_the_verbs_that_need_an_account_refuse_one_nobody_ever_saw(
    admin_operator_session: PersonaSession, verb: str
) -> None:
    """#2486 scenario 3. Binding an unseen account is allowed — that is
    pre-registration, the one deliberate asymmetry of the grammar. Detaching
    or excluding one is a typo: there is nothing to detach, and an excluded
    binding for an account no connector ever reported is a decision about
    nobody.

    Both verbs share `require_known_account`, so the pair is what proves the
    guard rather than one caller of it: a refactor that moves the check into
    `detach` alone would leave `exclude` minting journal rows for typos.
    """
    response = admin_operator_session.client.post(
        identity_path(f"/v1/resolution/{verb}"),
        json_body={"account": _account("never-observed-account")},
    )
    _refused(
        response,
        404,
        resource=f"{SCRATCH_SOURCE_TYPE}:{SCRATCH_SOURCE_ID}:never-observed-account",
    )


@pytest.mark.requires_seed("admin_operator", "dev_lead")
@pytest.mark.reliability
@pytest.mark.parametrize("unknown_side", ["source_person_id", "target_person_id"])
def test_merge_refuses_a_person_the_tenant_never_had(
    admin_operator_session: PersonaSession, stand_manifest: Manifest, unknown_side: str
) -> None:
    """#2486 scenario 3. A merge may not invent either of its two people.

    `bind` proves `require_known_person` for its own single target; merge
    calls the same guard twice, and the two calls are not one test. The
    target side is the one that would go unnoticed: an unknown SOURCE merges
    an empty set of accounts and looks like a harmless no-op, while an
    unknown TARGET would rebind every account of a real person onto an id
    nobody holds — accounts alive in the journal and attached to no one.

    Pure validation: the guard runs before any account is read, so the stand
    reads the same before and after.
    """
    lead = stand_manifest.fixture("dev_lead")
    body: dict[str, JsonValue] = {
        "source_person_id": lead.uuid,
        "target_person_id": lead.uuid,
    }
    body[unknown_side] = UNKNOWN_PERSON

    response = admin_operator_session.client.post(
        identity_path("/v1/resolution/merge"), json_body=body
    )
    # Naming the id is what makes the two cases distinct. Both sides answer
    # "person not found", so a merge that checked only its source would pass
    # the target case on the source's own refusal.
    _refused(response, 404, resource=UNKNOWN_PERSON)


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_a_cursor_issued_for_another_needle_is_refused(
    admin_operator_session: PersonaSession,
) -> None:
    """#2486 scenario 3. The account listing's cursor is bound to the search
    that issued it — the response schema says so in its own description
    ("Only valid for the query that issued it — narrowing `q` starts over").

    Resuming a narrowed search where a wider one left off would skip every
    account before that position, silently, and only for the operator who
    typed one more letter into the box. `/v1/persons` refuses it; this is the
    same rule on the surface where the accounts an operator is deciding about
    actually live.
    """
    first = admin_operator_session.client.get(ACCOUNT_SEARCH, params={"limit": 1})
    assert first.status_code == 200, f"{first.status_code} {first.text[:300]}"
    cursor = first.parse(AccountSearchResponse).next_cursor
    assert cursor, "a one-row page of the observed accounts must offer a next page"

    resumed = admin_operator_session.client.get(
        ACCOUNT_SEARCH, params={"limit": 1, "q": "nobody-by-this-name", "cursor": cursor}
    )
    _refused(resumed, 400, field="cursor")


def _verb_body(verb: str, person_id: str) -> dict[str, JsonValue]:
    if verb == "bind":
        return {"bindings": [{"account": _account("never-observed-account"), "person_id": person_id}]}
    if verb == "merge":
        return {"source_person_id": person_id, "target_person_id": UNKNOWN_PERSON}
    return {"account": _account("never-observed-account")}


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
@pytest.mark.parametrize("verb", [v for v, _ in CORRECTION_VERBS])
def test_a_correction_body_that_will_not_parse_is_refused_in_the_canonical_shape(
    admin_operator_session: PersonaSession, verb: str
) -> None:
    """#2486 scenario 3. A mistyped correction is answered in the shape the
    console can read.

    These four verbs took axum's own `Json`, while every other write route on
    the service takes the canonical extractor — so a body that would not parse
    answered below the canonical error layer, as `text/plain` with no problem
    document at all. The console extracts its message from that document, so
    an operator saw the generic failure text and no reason.
    """
    response = admin_operator_session.client.post(
        identity_path(f"/v1/resolution/{verb}"),
        content="{not json",
        headers={"Content-Type": "application/json"},
    )
    _refused(response, 400, field="body")


@pytest.mark.requires_seed("admin_operator", "dev_lead")
@pytest.mark.reliability
@pytest.mark.parametrize("verb", [v for v, _ in CORRECTION_VERBS])
def test_a_comment_past_the_cap_is_refused_before_the_correction_applies(
    admin_operator_session: PersonaSession, stand_manifest: Manifest, verb: str
) -> None:
    """#2486 scenario 3. The other free-text field an operator writes is
    bounded, at the ceiling `reason` already carries on the grant routes.

    Not a column limit — the journal stores JSON and would take any length.
    The point is that one free-text field on this service is not unbounded
    while its twin is. Refused before any lookup, so no write happens.
    """
    body = _verb_body(verb, stand_manifest.fixture("dev_lead").uuid)
    body["comment"] = "c" * 501

    response = admin_operator_session.client.post(
        identity_path(f"/v1/resolution/{verb}"), json_body=body
    )
    _refused(response, 400, field="comment")


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_an_account_search_needle_past_the_cap_is_refused(
    admin_operator_session: PersonaSession,
) -> None:
    """#2486 scenario 3. `/v1/persons` and `/v1/visible-persons` both bound
    their needle at 200 characters; this listing handed the raw one to the
    evidence reader."""
    response = admin_operator_session.client.get(ACCOUNT_SEARCH, params={"q": "n" * 201})
    _refused(response, 400, field="q")


@pytest.mark.requires_seed("admin_operator", "dev_lead")
@pytest.mark.reliability
@pytest.mark.parametrize("verb", [v for v, _ in CORRECTION_VERBS])
def test_a_correction_without_a_json_content_type_is_refused_on_the_media_type(
    admin_operator_session: PersonaSession, stand_manifest: Manifest, verb: str
) -> None:
    """#2486 scenario 7. Refused on the media type, never parsed.

    Held before the extractor swap too, since axum's own `Json` refuses on
    content type as well — so this is not a rule the swap introduced. It is
    here because the swap is exactly the change that could trade the 415 away
    for a parse attempt, and because a gateway is the other place a media type
    can be rewritten in flight: the in-process case cannot see that.
    """
    body = _verb_body(verb, stand_manifest.fixture("dev_lead").uuid)
    response = admin_operator_session.client.post(
        identity_path(f"/v1/resolution/{verb}"),
        content=json.dumps(body),
        headers={"Content-Type": "text/plain"},
    )
    assert response.status_code == 415, f"{response.status_code} {response.text[:300]}"


@pytest.mark.requires_seed("admin_operator", "dev_lead")
@pytest.mark.reliability
def test_a_bulk_bind_outside_its_bounds_is_refused(
    admin_operator_session: PersonaSession, stand_manifest: Manifest
) -> None:
    """#2486 scenario 7. The shapes a prepared matching table can arrive in
    wrongly: nothing to do, more than a human pasted, and one account claimed
    twice.

    All three are pinned in the in-process lanes already. They are here
    because scenario 7 says EVERY correction operation, and because these are
    the refusals a deployed path can change without the service moving: a
    gateway body-size limit turns the over-cap case into someone else's error
    long before `MAX_BULK_ITEMS` is consulted.
    """
    person = stand_manifest.fixture("dev_lead").uuid
    account = _account("never-observed-account")

    empty = admin_operator_session.client.post(
        identity_path("/v1/resolution/bind"), json_body={"bindings": [], "comment": "nothing"}
    )
    _refused(empty, 400, field="bindings")

    over_cap = admin_operator_session.client.post(
        identity_path("/v1/resolution/bind"),
        json_body={
            "bindings": [
                {"account": _account(f"never-observed-{index}"), "person_id": person}
                for index in range(1001)
            ]
        },
    )
    _refused(over_cap, 400, field="bindings")

    twice = admin_operator_session.client.post(
        identity_path("/v1/resolution/bind"),
        json_body={
            "bindings": [
                {"account": account, "person_id": person},
                {"account": account, "person_id": person},
            ]
        },
    )
    _refused(twice, 400, field="bindings")
