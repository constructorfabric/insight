"""What the connector answers before a sync is allowed to start.

`check_connection` is the only place that speaks to an operator, so each refusal
is asserted by what it says, not merely by being false: a check that fails
without naming the fix costs the same debugging time as no check at all.
"""

from collections.abc import Iterator
from typing import Any

import pytest
import requests
import requests_mock as rm_module
from source_claude_team_invoices.source import SourceClaudeTeamInvoices, load_stream_schema
from source_claude_team_invoices.streams import InvoiceLines

CONFIG = {
    "proxy_url": "https://proxy.example/",
    "proxy_auth_token": "proxy-token",
    "claude_org_id": "org_EXAMPLE",
    "insight_tenant_id": "11111111-1111-1111-1111-111111111111",
    "insight_source_id": "claude-team-invoices-1",
}
INVOICES_URL = f"https://proxy.example/api/stripe/{CONFIG['claude_org_id']}/invoices"
STRIPE_PING = "https://api.stripe.com/v1"


@pytest.fixture
def source() -> SourceClaudeTeamInvoices:
    return SourceClaudeTeamInvoices()


@pytest.fixture
def http() -> Iterator[rm_module.Mocker]:
    with rm_module.Mocker() as mocker:
        yield mocker


def test_the_stream_schema_ships_with_the_package() -> None:
    """`discover` reads it, so a missing file is a connector that cannot run."""
    schema = load_stream_schema("claude_team_invoice_lines")
    assert schema["properties"]["unique_key"] == {"type": "string"}
    assert "seat_unit_amount" in schema["properties"]


def test_the_source_offers_the_one_stream(source: SourceClaudeTeamInvoices) -> None:
    streams = source.streams(CONFIG)
    assert [type(s) for s in streams] == [InvoiceLines]


def test_an_empty_source_id_is_refused_before_any_request(
    source: SourceClaudeTeamInvoices, http: rm_module.Mocker
) -> None:
    ok, message = source.check_connection(None, dict(CONFIG, insight_source_id="  "))
    assert ok is False
    assert "insight_source_id" in message and "annotation" in message
    assert http.call_count == 0, "a config that cannot produce keys is refused without a call"


def test_an_unreachable_proxy_names_the_url(source: SourceClaudeTeamInvoices, http: rm_module.Mocker) -> None:
    http.get(INVOICES_URL, exc=requests.ConnectionError("no route"))
    ok, message = source.check_connection(None, CONFIG)
    assert ok is False
    assert "proxy unreachable at https://proxy.example" in message


def test_a_forbidden_listing_names_the_rotation_it_needs(
    source: SourceClaudeTeamInvoices, http: rm_module.Mocker
) -> None:
    http.get(INVOICES_URL, status_code=403)
    ok, message = source.check_connection(None, CONFIG)
    assert ok is False
    assert "not permitted to read invoices" in message
    assert "POST /admin/session-key" in message, "the refusal carries the fix"


def test_any_other_status_is_reported_with_its_code(source: SourceClaudeTeamInvoices, http: rm_module.Mocker) -> None:
    http.get(INVOICES_URL, status_code=502)
    ok, message = source.check_connection(None, CONFIG)
    assert ok is False
    assert "HTTP 502" in message


@pytest.mark.parametrize(
    ("body", "expected"),
    [
        ({"text": "<html>maintenance</html>"}, "not JSON"),
        ({"json": ["invoice"]}, "answered a list"),
        ({"json": {"data": []}}, "`invoices` array"),
        ({"json": {"invoices": 1}}, "`invoices` array"),
    ],
    ids=["not-json", "not-an-object", "no-invoices-field", "invoices-not-an-array"],
)
def test_a_200_that_is_not_an_invoice_listing_is_refused(
    source: SourceClaudeTeamInvoices, http: rm_module.Mocker, body: dict[str, Any], expected: str
) -> None:
    """A 200 proves the proxy answered, not that it answered this API.

    The array case matters most: a sync extends that value, so a scalar would
    fail mid-run after this check had already reported the source healthy.
    """
    http.get(INVOICES_URL, **body)
    ok, message = source.check_connection(None, CONFIG)
    assert ok is False
    assert expected in message


def test_blocked_egress_to_stripe_is_refused_even_when_the_proxy_is_healthy(
    source: SourceClaudeTeamInvoices, http: rm_module.Mocker
) -> None:
    """A green proxy with no egress yields invoices without prices — the silent
    emptiness this check exists to prevent."""
    http.get(INVOICES_URL, json={"invoices": []})
    http.get(STRIPE_PING, exc=requests.ConnectionError("blocked"))
    ok, message = source.check_connection(None, CONFIG)
    assert ok is False
    assert "api.stripe.com is not reachable" in message
    assert "every seat price would be missing" in message


def test_a_reachable_proxy_and_stripe_pass(source: SourceClaudeTeamInvoices, http: rm_module.Mocker) -> None:
    http.get(INVOICES_URL, json={"invoices": []})
    http.get(STRIPE_PING, status_code=401, json={"error": "unauthenticated"})
    assert source.check_connection(None, CONFIG) == (True, None)
