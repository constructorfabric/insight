"""The chain over HTTP, with every hop answered by a mock transport.

The unit under test is `InvoiceLines` itself — its pagination, its headers, the
credential it carries between hops and the row it writes when a hop fails. The
sibling suites exercise `build_records` with a stub in place of the network, so
nothing there sees a URL, a header or a status code; this is where those live.

Every request is matched or the test fails: `requests_mock` raises NoMockAddress
on an unregistered URL, so a hop that silently changes shape cannot pass.
"""

import base64
import json
import logging
from collections.abc import Iterator, Mapping, Sequence
from typing import Any

import pytest
import requests_mock as rm_module
from source_claude_team_invoices import streams as streams_module
from source_claude_team_invoices.streams import BOOTSTRAP_HOST, STRIPE_API, STRIPE_VERSION, InvoiceLines

CONFIG = {
    "proxy_url": "https://proxy.example/",
    "proxy_auth_token": "proxy-token",
    "claude_org_id": "org_EXAMPLE",
    "insight_tenant_id": "11111111-1111-1111-1111-111111111111",
    "insight_source_id": "claude-team-invoices-1",
}

ACCT = "acct_1EXAMPLE"
# base64 of `acct_1EXAMPLE,_entEXAMPLE,<rotating>` — the shape the vendor emits, so
# the connector can decode an invoice identity out of it.
_PAYLOAD = f"{ACCT},_entEXAMPLE,1785456000-n1"
TOKEN = "live_" + base64.urlsafe_b64encode(_PAYLOAD.encode()).decode().rstrip("=")
INVOICE_ID = "in_1EXAMPLE"
EPHEMERAL_KEY = "ek_live_EXAMPLESECRET"

INVOICES_URL = f"https://proxy.example/api/stripe/{CONFIG['claude_org_id']}/invoices"
BOOTSTRAP_URL = f"{BOOTSTRAP_HOST}/hosted_invoice_page/{ACCT}/{TOKEN}"
LINES_URL = f"{STRIPE_API}/invoices/{INVOICE_ID}/lines"


def wrapper_invoice(**over: Any) -> dict[str, Any]:
    """One row as the claude.ai wrapper reports it: money, no id, no lines."""
    base = {
        "hosted_invoice_url": f"https://invoice.stripe.com/i/{ACCT}/{TOKEN}?s=ap",
        "status": "paid",
        "created_ts": 1785456000,
        "currency": "usd",
        "total": 3300,
        "total_excluding_tax": 3000,
        "num_seats": 3,
        "payment_intent": "pi_EXAMPLE",
    }
    base.update(over)
    return base


def subscription_line(line_id: str = "il_standard", **over: Any) -> dict[str, Any]:
    base = {
        "id": line_id,
        "description": "3 x Example plan - Standard (at $10.00 / month)",
        "amount": 3000,
        "currency": "usd",
        "quantity": 3,
        "hosted_invoice_unit_amount": 1000,
        "hosted_invoice_product_name": "Example plan - Standard",
        "hosted_invoice_tier_label": "Standard",
        "parent": {"subscription_item_details": {"proration": False}},
        "period": {"start": 1785542400, "end": 1788220800},
    }
    base.update(over)
    return base


@pytest.fixture
def stream() -> InvoiceLines:
    return InvoiceLines(CONFIG)


@pytest.fixture
def http() -> Iterator[rm_module.Mocker]:
    with rm_module.Mocker() as mocker:
        yield mocker


def register_chain(
    http: rm_module.Mocker, lines_pages: Sequence[Mapping[str, Any]], bootstrap: Mapping[str, Any] | None = None
) -> None:
    """Answer all three hops: the wrapper list, the bootstrap, the line pages."""
    http.get(INVOICES_URL, json={"invoices": [wrapper_invoice()], "next_page": None})
    http.get(
        BOOTSTRAP_URL,
        json=bootstrap if bootstrap is not None else {"invoice_id": INVOICE_ID, "ephemeral_key": EPHEMERAL_KEY},
    )
    http.get(LINES_URL, [{"json": page} for page in lines_pages])


def test_the_chain_yields_the_invoice_row_and_one_record_per_line(stream: InvoiceLines, http: rm_module.Mocker) -> None:
    register_chain(http, [{"data": [subscription_line()], "has_more": False}])

    records = list(stream.read_records(sync_mode="full_refresh"))

    assert len(records) == 2
    own, line = records
    assert [r["chain_status"] for r in records] == ["ok", "ok"]
    assert [r["invoice_id"] for r in records] == [INVOICE_ID, INVOICE_ID]
    assert (own["line_id"], own["invoice_total_excluding_tax"]) == (None, 3000)
    assert (line["line_id"], line["seat_unit_amount"]) == ("il_standard", 1000)
    assert line["invoice_total_excluding_tax"] is None, "the invoice's money is on its own row"
    # The framework fields every bronze row carries (ADR-0004).
    for record in records:
        assert record["tenant_id"] == CONFIG["insight_tenant_id"]
        assert record["source_id"] == CONFIG["insight_source_id"]
        assert record["data_source"] == "insight_claude_team"
        assert record["unique_key"].startswith(f"{CONFIG['insight_tenant_id']}-{CONFIG['insight_source_id']}-")
    assert own["unique_key"] != line["unique_key"]


def test_each_hop_carries_the_credential_that_authorises_it(stream: InvoiceLines, http: rm_module.Mocker) -> None:
    register_chain(http, [{"data": [subscription_line()], "has_more": False}])

    list(stream.read_records(sync_mode="full_refresh"))

    by_host = {request.url.split("/")[2]: request for request in http.request_history}
    assert by_host["proxy.example"].headers["Authorization"] == f"Bearer {CONFIG['proxy_auth_token']}"
    # The bootstrap hop is the one the hosted URL's token authorises, so it
    # carries no header of its own.
    assert "Authorization" not in by_host["invoicedata.stripe.com"].headers
    lines_request = by_host["api.stripe.com"]
    assert lines_request.headers["Authorization"] == f"Bearer {EPHEMERAL_KEY}"
    assert lines_request.headers["Stripe-Version"] == STRIPE_VERSION
    assert lines_request.headers["Stripe-Account"] == ACCT


def test_line_pages_are_walked_until_the_endpoint_stops_offering_more(
    stream: InvoiceLines, http: rm_module.Mocker
) -> None:
    register_chain(
        http,
        [
            {"data": [subscription_line("il_first")], "has_more": True},
            {"data": [subscription_line("il_second")], "has_more": False},
        ],
    )

    records = list(stream.read_records(sync_mode="full_refresh"))

    assert [r["line_id"] for r in records] == [None, "il_first", "il_second"], "the invoice's own row leads"
    line_requests = [r for r in http.request_history if "api.stripe.com" in r.url]
    assert len(line_requests) == 2
    assert "starting_after=il_first" in line_requests[1].url, "the second page resumes after the first page's last id"


def test_the_chain_is_paced_between_invoices_but_not_before_the_first(
    stream: InvoiceLines, http: rm_module.Mocker, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The bootstrap host is undocumented and a run walks the whole history, so the
    chain is the one part whose request rate is ours to choose."""
    slept: list[float] = []
    monkeypatch.setattr(streams_module.time, "sleep", slept.append)

    second = wrapper_invoice(payment_intent="pi_SECOND")
    http.get(INVOICES_URL, json={"invoices": [wrapper_invoice(), second], "next_page": None})
    http.get(BOOTSTRAP_URL, json={"invoice_id": INVOICE_ID, "ephemeral_key": EPHEMERAL_KEY})
    http.get(LINES_URL, json={"data": [subscription_line()], "has_more": False})

    list(stream.read_records(sync_mode="full_refresh"))

    assert slept == [streams_module.CHAIN_DELAY_SECS], "two invoices, one pause, none before the first"


def test_a_hop_that_fails_leaves_a_gap_and_does_not_end_the_run(
    stream: InvoiceLines, http: rm_module.Mocker, caplog: pytest.LogCaptureFixture
) -> None:
    register_chain(http, [{"json": {}}], bootstrap={"ephemeral_key": EPHEMERAL_KEY})

    with caplog.at_level(logging.WARNING):
        records = list(stream.read_records(sync_mode="full_refresh"))

    assert [r["chain_status"] for r in records] == ["failed"]
    assert records[0]["invoice_total_excluding_tax"] == 3000, "the money stays on the ledger"
    assert records[0]["line_id"] is None and records[0]["seat_unit_amount"] is None


def test_the_ephemeral_key_reaches_neither_a_record_nor_a_log_line(
    stream: InvoiceLines, http: rm_module.Mocker, caplog: pytest.LogCaptureFixture
) -> None:
    """Over the real request path: the key rides a header, the token a URL.

    Only what the connector itself logs counts — `requests_mock` echoes every
    request URL at DEBUG, and asserting over that would test the mock.
    """
    register_chain(http, [{"data": [subscription_line()], "has_more": False}])

    with caplog.at_level(logging.DEBUG):
        records = list(stream.read_records(sync_mode="full_refresh"))

    ours = "\n".join(r.getMessage() for r in caplog.records if r.name.startswith("airbyte"))
    blob = json.dumps(records) + ours
    assert EPHEMERAL_KEY not in blob
    assert TOKEN not in blob, "the hosted-invoice token authorises the bootstrap hop and is a credential too"


def test_a_sync_remembers_what_the_chain_found_so_the_next_one_can_skip_it(
    stream: InvoiceLines, http: rm_module.Mocker
) -> None:
    """State is an index into bronze: the invoice id is the key its rows carry."""
    register_chain(http, [{"data": [subscription_line()], "has_more": False}])

    list(stream.read_records(sync_mode="incremental"))

    remembered = stream.state["enriched"]
    assert list(remembered) == [f"{ACCT},_entEXAMPLE"]
    assert remembered[f"{ACCT},_entEXAMPLE"]["invoice_id"] == INVOICE_ID


def test_a_remembered_invoice_costs_no_stripe_request(stream: InvoiceLines, http: rm_module.Mocker) -> None:
    """The saving, asserted by request count rather than by how long a run took."""
    stream.state = {
        "enriched": {f"{ACCT},_entEXAMPLE": {"invoice_id": INVOICE_ID, "period_start_ts": 1, "period_end_ts": 2}}
    }
    listing = http.get(INVOICES_URL, json={"invoices": [wrapper_invoice()], "next_page": None})

    records = list(stream.read_records(sync_mode="incremental"))

    assert listing.call_count == 1, "the listing is still read in full"
    stripe_calls = [r for r in http.request_history if r.netloc != "proxy.example"]
    assert stripe_calls == [], "neither the bootstrap nor the line hop was issued"
    assert [r["chain_status"] for r in records] == ["ok"], "its own row, and no line rows"
    assert records[0]["invoice_id"] == INVOICE_ID


def test_an_empty_state_chains_everything(stream: InvoiceLines, http: rm_module.Mocker) -> None:
    """The first sync after a deploy carries nothing, so it behaves as before."""
    assert stream.state == {"enriched": {}}
    register_chain(http, [{"data": [subscription_line()], "has_more": False}])

    records = list(stream.read_records(sync_mode="incremental"))

    assert len(records) == 2
    assert any(r.netloc == "invoicedata.stripe.com" for r in http.request_history)


def test_a_failed_chain_is_not_remembered_and_is_tried_again(
    stream: InvoiceLines, http: rm_module.Mocker, caplog: pytest.LogCaptureFixture
) -> None:
    """Otherwise a transient failure would freeze that invoice unenriched."""
    http.get(INVOICES_URL, json={"invoices": [wrapper_invoice()], "next_page": None})
    http.get(BOOTSTRAP_URL, status_code=502)

    with caplog.at_level(logging.WARNING):
        records = list(stream.read_records(sync_mode="incremental"))

    assert [r["chain_status"] for r in records] == ["failed"]
    assert stream.state == {"enriched": {}}


def test_state_carries_no_credential(stream: InvoiceLines, http: rm_module.Mocker) -> None:
    """It rides the same wire as a record, so the same rule applies to it."""
    register_chain(http, [{"data": [subscription_line()], "has_more": False}])

    list(stream.read_records(sync_mode="incremental"))

    blob = json.dumps(stream.state)
    assert EPHEMERAL_KEY not in blob
    assert TOKEN not in blob, "the token authorises the bootstrap hop"


def test_what_a_run_learns_does_not_change_what_that_run_chains(stream: InvoiceLines, http: rm_module.Mocker) -> None:
    """The map is read as it stood when the run began. Written to mid-run, an
    invoice would be skipped because an earlier record in the same run had just
    enriched it — letting a run's own output decide its own behaviour."""
    two = [wrapper_invoice(), wrapper_invoice(payment_intent="pi_2")]
    http.get(INVOICES_URL, json={"invoices": two, "next_page": None})
    bootstrap = http.get(BOOTSTRAP_URL, json={"invoice_id": INVOICE_ID, "ephemeral_key": EPHEMERAL_KEY})
    http.get(LINES_URL, json={"data": [subscription_line()], "has_more": False})

    list(stream.read_records(sync_mode="incremental"))

    assert bootstrap.call_count == 2, "both were chained; neither was skipped by the other"
