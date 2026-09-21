"""A seat's cap has three states and they must stay apart.

`credit_limits` on the roster is an override: absent means the member is
governed by the workspace default, an empty array means an override that
removes the cap, and a populated array means an override that sets one. A
reader that collapses any two of those cannot answer whether a seat is
capped, so the connector stores the vendor's value whole rather than
flattening it.

The seat lifecycle fields are here for the same reason. `deactivated_time` is
the only departure signal the source offers, and `pending_seat_type` the only
pre-settlement sign of a tier change; neither has a consumer yet, and neither
can gain one if the connector never stores them.
"""

from __future__ import annotations

import json

from config import ACCOUNT_ID, PROXY_URL, ChatGptTeamConfigBuilder
from connector_tests import HttpMocker, HttpRequest, HttpResponse, load_fixture, read_stream

_CONNECTOR = "ai/chatgpt-team"
_STREAM = "chatgpt_team_seats"
_USERS_URL = f"{PROXY_URL}/api/accounts/{ACCOUNT_ID}/users"

_CAP = {"enforcement_mode": "HARD_CAP", "limit": 5000, "limit_mode": "amount_credits"}


def _member(handle: str, **overrides) -> dict:
    return load_fixture(
        __file__,
        "seats_member.json",
        id=f"user-{handle}",
        account_user_id=f"account-user-{handle}",
        email=f"{handle}@example.com",
        name=handle,
        **overrides,
    )


def _page(members: list[dict]) -> HttpResponse:
    body = load_fixture(__file__, "seats_page.json")
    body["items"] = members
    body["total"] = len(members)
    return HttpResponse(body=json.dumps(body), status_code=200)


def _by_email(output) -> dict[str, dict]:
    return {r.record.data["email"]: r.record.data for r in output.records}


def test_every_cap_state_survives_the_read(http_mocker: HttpMocker) -> None:
    config = ChatGptTeamConfigBuilder().build()
    members = [
        _member("no-override", credit_limits=None),
        _member("cap-removed", credit_limits=[]),
        _member("cap-set", credit_limits=[_CAP]),
    ]

    http_mocker.get(HttpRequest(_USERS_URL, query_params={"query": "", "limit": "25"}), _page(members))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors, f"the read failed: {output.errors}"
    rows = _by_email(output)

    assert rows["no-override@example.com"].get("credit_limits") is None
    assert rows["cap-removed@example.com"]["credit_limits"] == [], (
        "an override that removes the cap was collapsed into 'no override'"
    )
    assert rows["cap-set@example.com"]["credit_limits"] == [_CAP]


def test_the_seat_lifecycle_fields_reach_bronze(http_mocker: HttpMocker) -> None:
    config = ChatGptTeamConfigBuilder().build()
    members = [_member("leaver", seat_type=None, pending_seat_type="prolite", deactivated_time="2026-08-30T12:00:00Z")]

    http_mocker.get(HttpRequest(_USERS_URL, query_params={"query": "", "limit": "25"}), _page(members))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors, f"the read failed: {output.errors}"
    row = _by_email(output)["leaver@example.com"]

    assert row["deactivated_time"] == "2026-08-30T12:00:00Z"
    assert row["pending_seat_type"] == "prolite"
    assert row["account_user_id"] == "account-user-leaver"
    # The join date is `created_time` upstream; the column is named added_at.
    assert row["added_at"] == "2026-01-05T09:00:00Z"
    # A null tier is the case the staging model resolves from the roster
    # first and the usage row second — it has to arrive as null, not ''.
    assert row.get("seat_type") is None
