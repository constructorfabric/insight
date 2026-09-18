"""A workspace role too low to read billing must not take the sync down with it.

The subscription endpoint answers 401 when the session's workspace role is below
account-admin. That is a permanent property of the installation, not a transient
fault, so the stream is IGNOREd: it commits nothing and every other stream still
syncs.

What IGNORE costs is observability, and these tests pin exactly how much. The
error_message reaches the connector's log stream and nothing else — there is no
structured field, no record and no state message carrying it, so nothing
downstream can query which of 401 / 403 / 404 occurred. The silence check in dbt
compares an instance against its own past for that reason, and cannot see an
installation that never read billing at all.
"""

from __future__ import annotations

from config import ORG_ID, PROXY_URL, ChatGptTeamConfigBuilder
from connector_tests import HttpMocker, HttpRequest, HttpResponse, read_stream
from freezegun import freeze_time

_CONNECTOR = "ai/chatgpt-team"
_STREAM = "chatgpt_team_subscription_usage"
_USAGE_URL = f"{PROXY_URL}/api/subscriptions/{ORG_ID}/usage"

_NOW = "2026-08-19T11:30:00Z"
# The manifest pins the billing window to a fixed [-30d, today]; it deliberately
# does not read config['start_date'].
_WINDOW = {"start_date": "2026-07-20", "end_date": "2026-08-19"}

_ROLE_MESSAGE = "the workspace role is below account-admin"


def _denied(status: int) -> HttpResponse:
    return HttpResponse(body='{"detail":"denied"}', status_code=status)


def test_a_role_that_cannot_read_billing_does_not_fail_the_sync(http_mocker: HttpMocker) -> None:
    config = ChatGptTeamConfigBuilder().build()

    http_mocker.get(HttpRequest(_USAGE_URL, query_params=_WINDOW), _denied(401))

    with freeze_time(_NOW):
        output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors, (
        f"401 on billing is a property of the installation, not a fault — it must not fail the sync: {output.errors}"
    )
    assert not output.records, "an IGNOREd response must commit nothing"


def test_the_reason_reaches_the_log_and_nothing_else(http_mocker: HttpMocker) -> None:
    """The one place the difference between 'no billing capability' and 'nothing to
    report' is written down. It is a log line: not a record, not a state message,
    and not anything the warehouse can be asked about later."""
    config = ChatGptTeamConfigBuilder().build()

    http_mocker.get(HttpRequest(_USAGE_URL, query_params=_WINDOW), _denied(401))

    with freeze_time(_NOW):
        output = read_stream(_CONNECTOR, _STREAM, config)

    logged = "\n".join(str(entry.log.message) for entry in output.logs)
    assert _ROLE_MESSAGE in logged, (
        "the 401 filter's error_message must name the role, or an operator cannot "
        f"tell a permission problem from an empty period: {logged[:2000]}"
    )
    assert not output.records, (
        "and it must stay a log line — a message that became a record would change what the stream means"
    )


def test_a_blank_or_gated_org_is_tolerated_the_same_way(http_mocker: HttpMocker) -> None:
    """403 and 404 were already tolerated; adding 401 must not change them."""
    config = ChatGptTeamConfigBuilder().build()

    http_mocker.get(HttpRequest(_USAGE_URL, query_params=_WINDOW), _denied(404))

    with freeze_time(_NOW):
        output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors, f"404 must stay tolerated: {output.errors}"
    assert not output.records
