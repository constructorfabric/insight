"""Operator corrections on the deployed identity service, read back from its journal.

Three HR records reach identity through the roster connector and persons-seed mints a
person for each. The admin operator then confirms, detaches, excludes, merges and
re-points those bindings; the module proves what each verb appended to the journal,
what the read surfaces report about it, and that the next seed run keeps the
operator's answer.
"""

from __future__ import annotations

import uuid
from dataclasses import dataclass
from typing import Any

import pytest
from insight_datapath import identity_journal, records
from insight_datapath.bindings import hashed_source_id, wire_account
from insight_datapath.clickhouse import query
from insight_datapath.connector_path import ConnectorPath
from insight_datapath.identity_journal import SYSTEM_AUTHOR, Decision
from insight_datapath.instance import InstanceConfig
from insight_datapath.subjects import Subjects
from insight_stand.api import ApiClient, ApiResponse, JsonValue, identity_path
from insight_stand.personas import PersonaSession
from insight_stand.scratch_identity import SCRATCH_SOURCE_ID, SCRATCH_SOURCE_TYPE, scratch_name

pytestmark = pytest.mark.fixture

ROSTER_SOURCE_TYPE = "bamboohr"
ROSTER_SOURCE_ID = "bamboohr-test"

BIND = identity_path("/v1/resolution/bind")
MERGE = identity_path("/v1/resolution/merge")
DETACH = identity_path("/v1/resolution/detach")
EXCLUDE = identity_path("/v1/resolution/exclude")
ATTENTION = identity_path("/v1/resolution/attention")
BY_EXTERNAL_ID = "/internal/persons/by-external-id"

MODULE_TAG = uuid.uuid4().hex[:10]

SPEC_PEOPLE = (("a", "Ann Anchor"), ("b", "Ben Branch"), ("c", "Cam Cedar"))

type Account = dict[str, str]
type Body = dict[str, JsonValue]


@dataclass(frozen=True)
class Subject:
    """One HR record's person, and the roster account persons-seed minted it under."""

    key: str
    person_id: str
    account: Account


@dataclass(frozen=True)
class People:
    a: Subject
    b: Subject
    c: Subject


def _record(substitutions: dict[str, str], slot: str, display_name: str) -> dict[str, Any]:
    first = display_name.split()[0].lower()
    return records.employee(
        substitutions=substitutions,
        key=f"e-{MODULE_TAG}-{slot}",
        email=f"{first}.{MODULE_TAG}@example.com",
        display_name=display_name,
    )


def _subject(row: dict[str, Any], person_ids: dict[str, str], source_id: str) -> Subject:
    key = str(row["id"])
    return Subject(
        key=key,
        person_id=person_ids[str(row["workEmail"]).strip().lower()],
        account=wire_account(ROSTER_SOURCE_TYPE, source_id, key),
    )


@pytest.fixture(scope="module")
def people(
    connector_path: ConnectorPath,
    substitutions: dict[str, str],
    subjects: Subjects,
    instance_cfg: InstanceConfig,
) -> People:
    """Three roster people beneath the lead, each minted a person by persons-seed."""
    rows = [_record(substitutions, slot, name) for slot, name in SPEC_PEOPLE]
    connector_path.build({"bronze_bamboohr.employees": rows})
    subjects.publish()

    person_ids = subjects.person_ids_of_records(rows)
    missing = [row["workEmail"] for row in rows if row["workEmail"] not in person_ids]
    assert not missing, f"persons-seed minted no person for {missing}"

    source_id = hashed_source_id(instance_cfg, ROSTER_SOURCE_ID)
    a, b, c = [_subject(row, person_ids, source_id) for row in rows]
    return People(a=a, b=b, c=c)


def _canonical(value: str) -> str:
    return str(uuid.UUID(value))


def _uuid(value: JsonValue) -> str:
    assert isinstance(value, str), f"not a uuid: {value!r}"
    return _canonical(value)


def _object(response: ApiResponse, what: str) -> Body:
    body = response.json()
    assert isinstance(body, dict), f"{what}: body is not an object: {response.text[:300]}"
    return body


def _applied(response: ApiResponse, what: str) -> Body:
    assert response.status_code == 200, f"{what}: {response.status_code} {response.text[:300]}"
    return _object(response, what)


def _problem(response: ApiResponse, status: int) -> None:
    assert response.status_code == status, f"{response.status_code} {response.text[:300]}"

    body = _object(response, f"problem {status}")
    title = body.get("title")
    assert body.get("status") == status, body
    assert isinstance(title, str) and title.strip(), body


def _entries(value: JsonValue, what: str) -> list[Body]:
    assert isinstance(value, list), f"{what} is not a list: {value!r}"
    entries = [entry for entry in value if isinstance(entry, dict)]
    assert len(entries) == len(value), f"{what} holds a non-object entry: {value!r}"
    return entries


def _by_account_id(value: JsonValue, what: str) -> dict[str, Body]:
    return {str(entry["account_id"]): entry for entry in _entries(value, what)}


def _scratch(tag: str) -> Account:
    return wire_account(SCRATCH_SOURCE_TYPE, SCRATCH_SOURCE_ID, scratch_name(tag))


def _bind(operator: ApiClient, account: Account, person_id: str, comment: str) -> ApiResponse:
    return operator.post(
        BIND,
        json_body={"bindings": [{"account": account, "person_id": person_id}], "comment": comment},
    )


def _detach(operator: ApiClient, account: Account, comment: str) -> ApiResponse:
    return operator.post(DETACH, json_body={"account": account, "comment": comment})


def _exclude(operator: ApiClient, account: Account, comment: str) -> ApiResponse:
    return operator.post(EXCLUDE, json_body={"account": account, "comment": comment})


def _history_of(operator: ApiClient, account: Account) -> ApiResponse:
    return operator.get(
        identity_path(
            f"/v1/resolution/accounts/{account['source']}/{account['source_id']}/{account['id']}"
        )
    )


def _accounts_of(operator: ApiClient, person_id: str) -> ApiResponse:
    return operator.get(identity_path(f"/v1/resolution/persons/{person_id}/accounts"))


def _decisions(cfg: InstanceConfig, tenant: str, account: Account) -> list[Decision]:
    return identity_journal.account_decisions(
        cfg,
        tenant=tenant,
        source_type=account["source"],
        source_id=account["source_id"],
        account_id=account["id"],
    )


def _mint_person(operator: ApiClient, account: Account, host: str) -> str:
    """A person nothing else owns: pre-register `account` on `host`, then detach it."""
    bound = _applied(
        _bind(operator, account, host, "datapath: pre-register before minting"), "bind"
    )
    assert bound["applied"] == 1, bound

    detached = _applied(_detach(operator, account, "datapath: mint"), "detach")
    return _uuid(detached["new_person_id"])


def test_a_persons_accounts_are_listed_with_who_bound_them(
    operator: ApiClient, people: People, run_tag: str
) -> None:
    """A person's matching table lists every account bound to them and tells the roster
    binding automation made from the one an operator added."""
    before = _applied(_accounts_of(operator, people.a.person_id), "accounts")
    roster = _by_account_id(before["accounts"], "accounts")

    assert _uuid(before["person_id"]) == people.a.person_id, before
    assert people.a.key in roster, before
    assert roster[people.a.key]["source"] == ROSTER_SOURCE_TYPE, roster[people.a.key]
    assert _uuid(roster[people.a.key]["source_id"]) == people.a.account["source_id"], roster
    assert roster[people.a.key]["bound_by_operator"] is False, roster[people.a.key]

    scratch = _scratch(f"listed-{run_tag}")
    bound = _applied(_bind(operator, scratch, people.a.person_id, "datapath: listed"), "bind")
    assert bound["applied"] == 1, bound

    after = _applied(_accounts_of(operator, people.a.person_id), "accounts")
    listed = _by_account_id(after["accounts"], "accounts")
    assert scratch["id"] in listed, after
    assert listed[scratch["id"]]["bound_by_operator"] is True, listed[scratch["id"]]
    assert listed[people.a.key]["bound_by_operator"] is False, listed[people.a.key]


def test_confirming_an_automatic_binding_records_the_operator(
    operator: ApiClient,
    operator_session: PersonaSession,
    instance_cfg: InstanceConfig,
    tenant: str,
    people: People,
) -> None:
    """Binding an account to the person automation already gave it appends the operator's
    decision; only a repeat of that decision is a no-op."""
    before = _decisions(instance_cfg, tenant, people.a.account)
    assert before, "persons-seed bound the roster account"
    assert before[0].author_person_id == SYSTEM_AUTHOR, before[0]

    confirm = _applied(
        _bind(operator, people.a.account, people.a.person_id, "datapath: confirm"), "confirm"
    )
    assert confirm["applied"] == 1, confirm

    after = _decisions(instance_cfg, tenant, people.a.account)
    assert len(after) == len(before) + 1, "the confirmation is a new observation"
    assert after[0].person_id == people.a.person_id, after[0]
    assert after[0].author_person_id == _canonical(operator_session.person.uuid), after[0]

    repeat = _applied(
        _bind(operator, people.a.account, people.a.person_id, "datapath: repeat"), "repeat"
    )
    assert repeat["applied"] == 0, repeat
    assert repeat["already_decided"] == 1, repeat
    assert _decisions(instance_cfg, tenant, people.a.account) == after, "no duplicate history"


def test_detach_moves_the_account_and_names_the_person_it_reached(
    operator: ApiClient, instance_cfg: InstanceConfig, tenant: str, people: People
) -> None:
    """Detach moves an account off whatever grouping it had and reports the person it
    actually reached."""
    before = _decisions(instance_cfg, tenant, people.b.account)

    detached = _applied(_detach(operator, people.b.account, "datapath: detach"), "detach")
    assert detached["applied"] == 1, detached
    new_person = _uuid(detached["new_person_id"])

    after = _decisions(instance_cfg, tenant, people.b.account)
    assert len(after) == len(before) + 1, "the detach is a new observation"
    assert after[0].person_id == new_person, "the account reached the reported person"


@pytest.mark.requires_service_principal
def test_an_excluded_account_no_longer_resolves_at_login(
    operator: ApiClient, service_client: ApiClient, people: People, run_tag: str
) -> None:
    """An excluded account is nobody everywhere: the login bootstrap answers not-found
    rather than a shared sentinel identity."""
    account = _scratch(f"bot-{run_tag}")
    lookup = {"source_type": account["source"], "external_id": account["id"]}
    bound = _applied(
        _bind(operator, account, people.c.person_id, "datapath: pre-register the bot"), "bind"
    )
    assert bound["applied"] == 1, bound

    resolves = service_client.get(BY_EXTERNAL_ID, params=lookup)
    assert resolves.status_code == 200, (
        f"a bound account resolves before the exclusion: {resolves.text[:300]}"
    )

    excluded = _applied(_exclude(operator, account, "datapath: not a person"), "exclude")
    assert excluded["applied"] == 1, excluded

    gone = service_client.get(BY_EXTERNAL_ID, params=lookup)
    _problem(gone, 404)


def test_the_queue_honours_the_limit_without_narrowing_the_rates(operator: ApiClient) -> None:
    """`limit` truncates the items an operator is handed; the rates describe every observed
    account either way."""
    full = _applied(operator.get(ATTENTION), "attention")
    capped = _applied(operator.get(ATTENTION, params={"limit": 1}), "attention?limit=1")

    assert len(_entries(capped["items"], "items")) <= 1, capped["items"]
    assert capped["rates"] == full["rates"], "the rates are not a page of the queue"


def test_merge_moves_every_account_of_the_absorbed_person(
    operator: ApiClient, instance_cfg: InstanceConfig, tenant: str, people: People, run_tag: str
) -> None:
    """Merge is the whole-person verb: every account of the source ends up on the survivor,
    appended over a history the merge leaves intact."""
    first = _scratch(f"merge-a-{run_tag}")
    second = _scratch(f"merge-b-{run_tag}")
    source = _mint_person(operator, first, people.c.person_id)
    survivor = _mint_person(operator, _scratch(f"merge-keep-{run_tag}"), people.c.person_id)
    joined = _applied(_bind(operator, second, source, "datapath: second account"), "bind")
    assert joined["applied"] == 1, joined
    before = {
        account["id"]: _decisions(instance_cfg, tenant, account) for account in (first, second)
    }

    merged = _applied(
        operator.post(
            MERGE,
            json_body={
                "source_person_id": source,
                "target_person_id": survivor,
                "comment": "datapath: one human",
            },
        ),
        "merge",
    )
    assert merged["applied"] == 2, merged

    for account in (first, second):
        after = _decisions(instance_cfg, tenant, account)
        assert after[0].person_id == survivor, f"{account['id']} did not reach the survivor"
        assert after[1:] == before[account["id"]], f"{account['id']}: the merge rewrote its history"


def test_bind_refuses_a_person_the_tenant_never_had(operator: ApiClient, run_tag: str) -> None:
    """A correction may not invent its target: binding to an unknown person is refused, not
    recorded."""
    response = _bind(
        operator, _scratch(f"stranger-{run_tag}"), str(uuid.uuid4()), "datapath: unknown person"
    )

    _problem(response, 404)


def test_an_accounts_history_names_every_decision_and_its_author(
    operator: ApiClient, operator_session: PersonaSession, people: People, run_tag: str
) -> None:
    """The explain surface answers why an account belongs to whom: the binding in force plus
    each decision behind it, newest first, marked human or automatic."""
    account = _scratch(f"history-{run_tag}")
    person = _mint_person(operator, account, people.c.person_id)
    author = _canonical(operator_session.person.uuid)

    body = _applied(_history_of(operator, account), "account history")
    history = _entries(body["history"], "history")

    assert body["account_id"] == account["id"], body
    assert _uuid(body["person_id"]) == person, "the binding in force is the newest decision"
    assert [entry["reason"] for entry in history] == ["operator-detach", "operator-bind"], history
    assert all(entry["by_operator"] is True for entry in history), history
    assert all(_uuid(entry["author_person_id"]) == author for entry in history), history


def test_an_unknown_accounts_history_is_empty_rather_than_missing(operator: ApiClient) -> None:
    """An account nobody has bound is a legitimate question with an empty answer, not a 404."""
    body = _applied(_history_of(operator, _scratch("never-seen")), "account history")

    assert body["person_id"] is None, body
    assert body["history"] == [], body


def test_a_person_with_no_accounts_lists_none(operator: ApiClient) -> None:
    """A person id the journal never bound an account to answers with an empty table."""
    body = _applied(_accounts_of(operator, str(uuid.uuid4())), "accounts")

    assert body["accounts"] == [], body


def test_an_operator_decision_survives_the_next_seed_run(
    operator: ApiClient,
    operator_session: PersonaSession,
    subjects: Subjects,
    instance_cfg: InstanceConfig,
    tenant: str,
    people: People,
) -> None:
    """Automation binds an account, an operator moves it to another person, automation runs
    again and keeps the operator's decision, in the journal and in the mirror analytics reads."""
    author = _canonical(operator_session.person.uuid)
    origin = _decisions(instance_cfg, tenant, people.a.account)
    assert origin[-1].author_person_id == SYSTEM_AUTHOR, "the binding began as automation's"

    moved = _applied(
        _bind(operator, people.a.account, people.b.person_id, "datapath: one human"), "bind"
    )
    assert moved["applied"] == 1, moved
    subjects.publish()

    after = _decisions(instance_cfg, tenant, people.a.account)
    assert after[0].person_id == people.b.person_id, "the seed re-derived the overruled binding"
    assert after[0].author_person_id == author, "the surviving decision is still the operator's"

    mirrored = query(
        instance_cfg,
        "SELECT toString(person_id), toString(author_person_id)"
        " FROM identity.identity_persons"
        f" WHERE value_type = 'id' AND insight_source_type = '{ROSTER_SOURCE_TYPE}'"
        f"   AND toString(insight_source_id) = '{people.a.account['source_id']}'"
        f"   AND value_effective = '{people.a.key}'"
        " ORDER BY created_at DESC, id DESC LIMIT 1",
    )
    assert mirrored, "the correction never reached the mirror the analytics resolver reads"
    assert str(mirrored[0][0]) == people.b.person_id, "the mirror names the overruled person"
    assert str(mirrored[0][1]) == author, "the mirrored decision lost its author"
