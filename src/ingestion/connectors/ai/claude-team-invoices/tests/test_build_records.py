"""Degradation rules of a run, exercised without a socket.

`build_records` is handed a `fetch_lines` that returns or raises; every rule
about what reaches bronze when a hop fails is decided here, so none of it needs
a network, a cluster or the CDK to verify.
"""

import logging

import pytest
from source_claude_team_invoices.stripe_chain import (
    CHAIN_FAILED,
    CHAIN_OK,
    CHAIN_UNPARSABLE,
    StripeChainError,
    UrlFormatDrift,
    build_records,
    unique_key_parts,
)
from tests.test_stripe_chain import EXTRA_USAGE_LINE, SUBSCRIPTION_LINE

GOOD_URL = "https://invoice.stripe.com/i/acct_1ABC/live_TOKEN?s=ap"
EPHEMERAL_KEY = "ek_live_super_secret_value"


def invoice(url=GOOD_URL, **over):
    base = {
        "hosted_invoice_url": url,
        "status": "paid",
        "created_ts": 1756771200,
        "currency": "usd",
        "total": 1595000,
        "total_excluding_tax": 1550000,
        "num_seats": 18,
        "payment_intent": "pi_1",
    }
    base.update(over)
    return base


def lines_ok(acct, token):
    return "in_1ABC", [SUBSCRIPTION_LINE, EXTRA_USAGE_LINE]


def lines_raise(acct, token):
    # The key would be in scope here in production; the message must not carry it.
    raise StripeChainError("bootstrap response carried no invoice_id/ephemeral_key")


def test_an_enriched_invoice_yields_one_record_per_line():
    records = list(build_records([invoice()], lines_ok))
    assert len(records) == 2
    assert {r["chain_status"] for r in records} == {CHAIN_OK}
    assert [r["seat_unit_amount"] for r in records] == [12500, None]
    # Invoice-level facts ride along on every line.
    assert all(r["invoice_total_excluding_tax"] == 1550000 for r in records)


def test_an_unparsable_url_keeps_the_money_and_marks_the_gap():
    # Alongside a healthy invoice: one bad URL out of a set is a data gap, while
    # a set that is entirely bad is drift — see the two boundary tests below.
    records = list(build_records([invoice(url="https://elsewhere.example/i/a/b"), invoice()], lines_ok))
    row = next(r for r in records if r["chain_status"] == CHAIN_UNPARSABLE)
    assert row["chain_status"] == CHAIN_UNPARSABLE
    assert row["invoice_total_excluding_tax"] == 1550000, "the ledger survives"
    assert row["seat_unit_amount"] is None and row["line_id"] is None


def test_a_failed_chain_does_not_stop_the_run(caplog):
    invoices = [invoice(payment_intent="pi_bad"), invoice(payment_intent="pi_good")]
    calls = {"n": 0}

    def flaky(acct, token):
        calls["n"] += 1
        if calls["n"] == 1:
            raise StripeChainError("stripe said no")
        return lines_ok(acct, token)

    with caplog.at_level(logging.WARNING):
        records = list(build_records(invoices, flaky))

    assert [r["chain_status"] for r in records] == [CHAIN_FAILED, CHAIN_OK, CHAIN_OK]
    assert records[0]["invoice_total_excluding_tax"] == 1550000
    assert "stripe chain failed" in caplog.text


def test_a_run_of_unparsable_urls_fails_instead_of_writing_priceless_rows():
    invoices = [invoice(url="https://elsewhere.example/x") for _ in range(3)] + [invoice()]
    with pytest.raises(UrlFormatDrift) as raised:
        list(build_records(invoices, lines_ok))
    assert "3 of 4" in str(raised.value)


def test_a_single_bad_url_among_many_is_tolerated():
    invoices = [invoice(url="https://elsewhere.example/x")] + [invoice() for _ in range(9)]
    records = list(build_records(invoices, lines_ok))
    assert sum(1 for r in records if r["chain_status"] == CHAIN_UNPARSABLE) == 1
    assert sum(1 for r in records if r["chain_status"] == CHAIN_OK) == 18


def test_an_empty_invoice_list_is_not_drift():
    assert list(build_records([], lines_ok)) == []


def test_the_drift_guard_is_a_majority_not_a_single_failure():
    """Half the set failing is tolerated; more than half is not."""
    two = [invoice(url="bad"), invoice()]
    assert len(list(build_records(two, lines_ok))) == 3, "1 of 2 is not yet drift"

    with pytest.raises(UrlFormatDrift):
        list(build_records([invoice(url="bad")], lines_ok))


def test_no_record_carries_the_ephemeral_key():
    """The key authorises two hops and must not reach a row, at any status."""

    def lines_with_key(acct, token):
        # Shaped like a real line set; the key is deliberately in scope.
        assert EPHEMERAL_KEY
        return "in_1ABC", [SUBSCRIPTION_LINE]

    records = list(build_records([invoice(), invoice(), invoice(url="bad")], lines_with_key))
    blob = repr(records)
    assert EPHEMERAL_KEY not in blob
    assert "ek_" not in blob


def test_key_parts_identify_a_line_by_stripe_ids_and_a_gap_by_the_wrapper():
    records = list(build_records([invoice(url="bad"), invoice()], lines_ok))
    fallback = next(r for r in records if r["chain_status"] == CHAIN_UNPARSABLE)
    enriched = next(r for r in records if r["chain_status"] == CHAIN_OK)
    assert unique_key_parts(enriched) == ("in_1ABC", "il_premium")
    assert unique_key_parts(fallback) == (CHAIN_UNPARSABLE, 1756771200, "pi_1")


def test_two_lines_of_one_invoice_get_different_keys():
    records = list(build_records([invoice()], lines_ok))
    assert unique_key_parts(records[0]) != unique_key_parts(records[1])
