"""Mock-server tests for the opt-in `calendar_events` stream.

The stream lists enabled users, then reads each user's calendarView over the 27
finished UTC days before today. It reads nothing unless `m365_calendar` is
"true". Only the number of other people invited is kept, never their addresses.
A user without a mailbox, or a mailbox an application access policy excludes, is
skipped; any other denial fails the sync as a configuration error. The calendar
stays out of the connection check, which would fail on a stream with no partitions.
"""

from __future__ import annotations

import logging

import freezegun
from airbyte_cdk.models import Status
from config import (
    FROZEN_NOW,
    GRAPH_URL,
    M365ConfigBuilder,
    calendar_request,
    graph_error,
    mock_token,
    page,
    users_request,
)
from connector_tests import HttpMocker, HttpRequest, assert_records_conform, get_source, load_fixture, read_stream

_STREAM = "calendar_events"
_CONNECTOR = "collaboration/m365"

ALICE_ID = "00000000-0000-0000-0000-0000000000a1"
BOB_ID = "00000000-0000-0000-0000-0000000000b2"


def _alice() -> dict:
    return load_fixture(__file__, "user.json")


def _bob() -> dict:
    return load_fixture(__file__, "user.json", id=BOB_ID, userPrincipalName="bob@example.com", mail=None)


def _event(**overrides) -> dict:
    return load_fixture(__file__, "event.json", **overrides)


@freezegun.freeze_time(FROZEN_NOW)
def test_calendar_is_not_read_unless_switched_on(http_mocker: HttpMocker) -> None:
    output = read_stream(_CONNECTOR, _STREAM, M365ConfigBuilder().build())

    assert output.records == [], "the calendar must stay unread by default"
    assert not output.errors, "an unread calendar is not an error"


@freezegun.freeze_time(FROZEN_NOW)
def test_events_keep_the_invitee_count_and_drop_addresses(http_mocker: HttpMocker) -> None:
    mock_token(http_mocker)
    http_mocker.get(users_request(), page([_alice()]))
    http_mocker.get(calendar_request(ALICE_ID), page([_event()]))

    output = read_stream(_CONNECTOR, _STREAM, M365ConfigBuilder().with_calendar().build())

    assert len(output.records) == 1
    record = output.records[0].record.data
    assert record["other_invitees"] == 2, "self (by mail, any case) and the room are not other people"
    assert record["response"] == "accepted"
    assert record["start_time"] == "2026-06-15T09:00:00.0000000"
    assert record["end_time"] == "2026-06-15T09:30:00.0000000"
    assert record["user_id"] == ALICE_ID
    assert record["userPrincipalName"] == "alice@example.com"
    assert record["unique_key"] == f"test-tenant-test-source-{ALICE_ID}-AAMkAGI-synthetic-event-1"
    assert "attendees" not in record, "invitee addresses must never reach bronze"
    assert_records_conform(output.records, _CONNECTOR, _STREAM)


@freezegun.freeze_time(FROZEN_NOW)
def test_next_page_follows_the_link_alone(http_mocker: HttpMocker) -> None:
    next_link = f"{GRAPH_URL}/users/{ALICE_ID}/calendarView?$skiptoken=page2"
    mock_token(http_mocker)
    http_mocker.get(users_request(), page([_alice()]))
    http_mocker.get(calendar_request(ALICE_ID), page([_event()], next_link=next_link))
    http_mocker.get(
        HttpRequest(f"{GRAPH_URL}/users/{ALICE_ID}/calendarView", query_params={"$skiptoken": "page2"}),
        page([_event(id="AAMkAGI-synthetic-event-2")]),
    )

    output = read_stream(_CONNECTOR, _STREAM, M365ConfigBuilder().with_calendar().build())

    assert [r.record.data["id"] for r in output.records] == ["AAMkAGI-synthetic-event-1", "AAMkAGI-synthetic-event-2"]


@freezegun.freeze_time(FROZEN_NOW)
def test_unreachable_mailboxes_are_skipped(http_mocker: HttpMocker) -> None:
    carol_id = "00000000-0000-0000-0000-0000000000c3"
    mock_token(http_mocker)
    http_mocker.get(
        users_request(),
        page(
            [_alice(), _bob(), load_fixture(__file__, "user.json", id=carol_id, userPrincipalName="carol@example.com")]
        ),
    )
    http_mocker.get(
        calendar_request(ALICE_ID),
        graph_error(
            404, "MailboxNotEnabledForRESTAPI", "The mailbox is either inactive, soft-deleted, or is hosted on-premise."
        ),
    )
    http_mocker.get(
        calendar_request(BOB_ID),
        graph_error(
            403,
            "ErrorAccessDenied",
            "Access to OData is disabled: [RAOP] : Blocked by tenant configured AppOnly AccessPolicy settings.",
        ),
    )
    http_mocker.get(calendar_request(carol_id), page([_event()]))

    output = read_stream(_CONNECTOR, _STREAM, M365ConfigBuilder().with_calendar().build())

    assert [r.record.data["user_id"] for r in output.records] == [carol_id], "only the readable mailbox yields events"
    assert not output.errors, "a skipped mailbox must not fail the sync"


@freezegun.freeze_time(FROZEN_NOW)
def test_missing_calendar_permission_fails_the_sync(http_mocker: HttpMocker) -> None:
    mock_token(http_mocker)
    http_mocker.get(users_request(), page([_alice()]))
    http_mocker.get(
        calendar_request(ALICE_ID),
        graph_error(403, "ErrorAccessDenied", "Access is denied. Check credentials and try again."),
    )

    output = read_stream(_CONNECTOR, _STREAM, M365ConfigBuilder().with_calendar().build(), expecting_exception=True)

    assert output.records == []
    assert any("Calendars.ReadBasic.All" in str(e) for e in output.errors), (
        "the failure must name the missing permission"
    )


@freezegun.freeze_time(FROZEN_NOW)
def test_connection_check_passes_with_the_calendar_off(http_mocker: HttpMocker) -> None:
    config = M365ConfigBuilder().build()
    mock_token(http_mocker)
    http_mocker.get(
        HttpRequest(
            "https://graph.microsoft.com/beta/reports/getEmailActivityUserDetail(date=2026-06-04)",
            query_params={"$format": "application/json"},
        ),
        page([]),
    )

    result = get_source(_CONNECTOR, config).check(logging.getLogger("test"), config)

    assert result.status == Status.SUCCEEDED, f"an unused calendar must not fail the check: {result.message}"
