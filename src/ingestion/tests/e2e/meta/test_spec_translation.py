"""The email/person-id translation a spec's requests and responses pass through."""

from __future__ import annotations

from lib.spec_runner import all_persona_emails, email_of_person, person_ids_for, translate

ALICE = "alice@example.com"
BOB = "bob@example.com"


def test_spellings_of_one_address_share_a_person() -> None:
    ids = person_ids_for([ALICE, " Alice@Example.com", BOB], {})
    assert ids[ALICE] == ids[" Alice@Example.com"]
    assert ids[BOB] != ids[ALICE]


def test_a_declared_alias_resolves_to_the_canonical_person() -> None:
    ids = person_ids_for([ALICE, "alice.dev@example.com"], {ALICE: ["alice.dev@example.com"]})
    assert ids["alice.dev@example.com"] == ids[ALICE]


def test_every_spelling_translates_back_to_the_address_a_test_names() -> None:
    """A response carries person ids; a test compares against the canonical email, so
    an untidy bronze spelling and a declared alias must both resolve to it."""
    emails = [ALICE, " Alice@Example.com", "  Alice.Caps@Example.COM", BOB]
    aliases = {ALICE: ["alice.caps@example.com"]}
    ids = person_ids_for(emails, aliases)
    to_email = email_of_person(ids, aliases)
    assert to_email[ids[" Alice@Example.com"]] == ALICE
    assert to_email[ids["  Alice.Caps@Example.COM"]] == ALICE
    assert to_email[ids[BOB]] == BOB


def test_translation_reaches_every_string_in_a_nested_payload() -> None:
    payload = {"metrics": [{"values": [{"entity_id": ALICE, "value": 1}]}], "ids": [ALICE, BOB]}
    swapped = translate(payload, {ALICE: "id-a", BOB: "id-b"})
    assert swapped == {"metrics": [{"values": [{"entity_id": "id-a", "value": 1}]}], "ids": ["id-a", "id-b"]}


def test_every_address_the_bronze_mentions_is_bound() -> None:
    """Peer pools are seeded as data and never requested, so binding follows the seed
    rather than the request."""
    bronze = {
        "bronze_x.people": [{"workEmail": ALICE, "manager": {"email": BOB}}],
        "bronze_x.usage": [{"userEmail": " Alice@Example.com", "note": "no address here"}],
    }
    assert all_persona_emails(bronze) == sorted({ALICE, BOB, " Alice@Example.com".strip()})
