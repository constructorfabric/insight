"""Rules of the Stripe chain, exercised without a network call.

Fixtures carry the shapes the vendor emits, with synthetic values: a monthly
subscription line, a proration credit from a mid-period seat change, and a
prepaid extra-usage purchase.
"""

from collections.abc import Mapping
from typing import Any

import pytest
from source_claude_team_invoices.stripe_chain import (
    CATEGORY_OVERUSAGE,
    CATEGORY_SUBSCRIPTIONS,
    StripeChainError,
    classify_line,
    is_proration,
    parse_hosted_invoice_url,
    read_bootstrap,
    seat_unit_amount,
    shape_line,
)

SUBSCRIPTION_LINE = {
    "id": "il_standard",
    "description": "3 x Example plan - Standard (at $10.00 / month)",
    "amount": 3000,
    "currency": "usd",
    "quantity": 3,
    "hosted_invoice_unit_amount": 1000,
    "hosted_invoice_product_name": "Example plan - Standard",
    "hosted_invoice_tier_label": "Standard",
    "parent": {"subscription_item_details": {"proration": False}},
    "period": {"start": 1754092800, "end": 1756771200},
}

PRORATION_CREDIT = {
    "id": "il_unused",
    "description": "Unused time on 3 x Example plan - Standard",
    "amount": -1500,
    "quantity": 3,
    "hosted_invoice_unit_amount": None,
    "parent": {"subscription_item_details": {"proration": True}},
}

EXTRA_USAGE_LINE = {
    "id": "il_prepaid",
    "description": "Prepaid extra usage, Example plan",
    "amount": 2000,
    "quantity": 1,
    "hosted_invoice_unit_amount": 2000,
    "parent": {"invoice_item_details": {}},
}


@pytest.mark.parametrize(
    "url, acct, token",
    [
        ("https://invoice.stripe.com/i/acct_1ABC/live_XYZ-9?s=ap", "acct_1ABC", "live_XYZ-9"),
        ("https://invoice.stripe.com/i/acct_1ABC/test_XYZ", "acct_1ABC", "test_XYZ"),
        ("https://invoice.stripe.com/i/acct_1ABC/live_XYZ#frag", "acct_1ABC", "live_XYZ"),
    ],
)
def test_hosted_url_yields_account_and_token(url: str, acct: str, token: str) -> None:
    ref = parse_hosted_invoice_url(url)
    assert ref is not None, f"should parse: {url!r}"
    assert (ref.acct, ref.token) == (acct, token)


@pytest.mark.parametrize(
    "url",
    [
        None,
        "",
        "https://invoice.stripe.com/i/acct_1ABC",
        # A link on another host must not contribute path segments we then put
        # into a request URL, however well-formed the rest of it looks.
        "https://example.com/i/acct_1/live_2",
        "http://invoice.stripe.com/i/acct_1/live_2",
    ],
)
def test_unparsable_url_is_absent_not_an_error(url: str) -> None:
    assert parse_hosted_invoice_url(url) is None, f"should not parse: {url!r}"


def test_category_comes_from_the_parent_not_the_description() -> None:
    assert classify_line(SUBSCRIPTION_LINE) == CATEGORY_SUBSCRIPTIONS
    assert classify_line(EXTRA_USAGE_LINE) == CATEGORY_OVERUSAGE
    # A subscription-looking description under an invoice-item parent stays overusage.
    disguised = dict(EXTRA_USAGE_LINE, description="3 x Example plan - Standard")
    assert classify_line(disguised) == CATEGORY_OVERUSAGE


def test_unknown_parent_falls_back_to_overusage() -> None:
    assert classify_line({"parent": {}}) == CATEGORY_OVERUSAGE
    assert classify_line({}) == CATEGORY_OVERUSAGE


def test_only_a_settled_subscription_line_prices_a_seat() -> None:
    assert seat_unit_amount(SUBSCRIPTION_LINE) == 1000
    assert seat_unit_amount(PRORATION_CREDIT) is None, "a proration carries no seat price"
    assert seat_unit_amount(EXTRA_USAGE_LINE) is None, "extra usage is not a seat"


def test_a_subscription_line_the_vendor_left_unpriced_yields_absence() -> None:
    unpriced = dict(SUBSCRIPTION_LINE, hosted_invoice_unit_amount=None)
    assert seat_unit_amount(unpriced) is None


@pytest.mark.parametrize("value", [True, False])
def test_a_boolean_unit_amount_is_absence_not_a_price(value: bool) -> None:
    """`bool` is an `int` in Python, so an unguarded check prices a seat at 1."""
    assert seat_unit_amount(dict(SUBSCRIPTION_LINE, hosted_invoice_unit_amount=value)) is None


def test_proration_is_read_from_the_structural_flag() -> None:
    assert is_proration(PRORATION_CREDIT)
    assert not is_proration(SUBSCRIPTION_LINE)
    assert not is_proration(EXTRA_USAGE_LINE)


def test_shape_keeps_the_charged_period_not_the_invoice_date() -> None:
    row = shape_line(SUBSCRIPTION_LINE, "inv-key")
    assert row["period_start_ts"] == 1754092800
    assert row["period_end_ts"] == 1756771200
    assert row["seat_unit_amount"] == 1000
    assert row["tier_label"] == "Standard"
    assert row["invoice_key"] == "inv-key"


def test_shape_of_a_proration_keeps_the_money_and_drops_the_price() -> None:
    row = shape_line(PRORATION_CREDIT, "inv-key")
    assert row["amount"] == -1500, "a credit stays on the ledger"
    assert row["category"] == CATEGORY_SUBSCRIPTIONS
    assert row["is_proration"] is True
    assert row["seat_unit_amount"] is None


def test_bootstrap_returns_the_pair() -> None:
    assert read_bootstrap({"invoice_id": "in_1ABC", "ephemeral_key": "ek_x"}) == ("in_1ABC", "ek_x")


@pytest.mark.parametrize(
    "payload", [{}, {"invoice_id": "in_1ABC"}, {"ephemeral_key": "ek_x"}, {"invoice_id": "", "ephemeral_key": "ek_x"}]
)
def test_incomplete_bootstrap_fails_loudly(payload: Mapping[str, Any]) -> None:
    with pytest.raises(StripeChainError):
        read_bootstrap(payload)


@pytest.mark.parametrize("invoice_id", ["../etc", "sub_1ABC", "in_ABC/../x", "in_ABC?x=1"])
def test_bootstrap_rejects_an_invoice_id_that_is_not_one(invoice_id: str) -> None:
    with pytest.raises(StripeChainError):
        read_bootstrap({"invoice_id": invoice_id, "ephemeral_key": "ek_x"})
