"""Rules of the Stripe chain, exercised without a network call.

Fixtures carry the shapes the vendor emits, with synthetic values: a monthly
subscription line, a proration credit from a mid-period seat change, and a
prepaid extra-usage purchase.
"""

import base64
from collections.abc import Mapping
from typing import Any

import pytest
from source_claude_team_invoices import stripe_chain as chain_module
from source_claude_team_invoices.stripe_chain import (
    CATEGORY_OVERUSAGE,
    CATEGORY_SUBSCRIPTIONS,
    PriceRef,
    StripeChainError,
    classify_line,
    is_proration,
    parse_hosted_invoice_url,
    price_details,
    read_bootstrap,
    seat_unit_amount,
    shape_line,
    stable_invoice_ref,
    unreadable_seat_prices,
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


def _hosted_url(payload: bytes, acct: str = "acct_1ABC") -> str:
    """A hosted URL whose path segment is base64url of `payload`, unpadded."""
    return f"https://invoice.stripe.com/i/{acct}/live_{base64.urlsafe_b64encode(payload).decode().rstrip('=')}?s=ap"


# The rotating part of a payload: a timestamp followed by bytes that are not text.
BINARY_NONCE = b"1756771200\xff\xfe\x01"


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


def test_the_identity_is_read_past_a_nonce_that_is_not_text() -> None:
    """Only the two leading fields identify the invoice, and they are ASCII; the
    nonce behind them is raw bytes, so the payload is no single string."""
    assert stable_invoice_ref(_hosted_url(b"acct_1ABC,_ent1ABC," + BINARY_NONCE)) == "acct_1ABC,_ent1ABC"


def test_one_invoice_keeps_one_identity_as_its_nonce_rotates() -> None:
    calls = [b"acct_1ABC,_ent1ABC,1756771200\xff\x01", b"acct_1ABC,_ent1ABC,1756800000\xff\x02"]
    assert {stable_invoice_ref(_hosted_url(payload)) for payload in calls} == {"acct_1ABC,_ent1ABC"}


@pytest.mark.parametrize(
    "payload",
    [
        b"acct_1ABC",
        b"acct_1ABC,," + BINARY_NONCE,
        b"1ABC,_ent1ABC," + BINARY_NONCE,
        b"acct_1ABC,_ent\xff1ABC," + BINARY_NONCE,
    ],
)
def test_no_identity_unless_both_leading_fields_are_readable(payload: bytes) -> None:
    assert stable_invoice_ref(_hosted_url(payload)) is None, f"should carry no identity: {payload!r}"


def test_a_segment_that_is_not_base64_carries_no_identity() -> None:
    assert stable_invoice_ref("https://invoice.stripe.com/i/acct_1ABC/live_ABCDE") is None


def test_a_line_names_its_tier_by_the_vendors_own_price_and_product() -> None:
    """The display name is localised marketing copy; these two are catalogue ids."""
    line = dict(SUBSCRIPTION_LINE, pricing={"price_details": {"price": "price_1ABC", "product": "prod_1ABC"}})
    assert price_details(line) == PriceRef("price_1ABC", "prod_1ABC")
    shaped = shape_line(line, "in_1ABC")
    assert (shaped["price_id"], shaped["product_id"]) == ("price_1ABC", "prod_1ABC")


@pytest.mark.parametrize(
    "pricing",
    [None, {}, {"price_details": None}, {"price_details": {}}, {"price_details": {"price": "", "product": ""}}],
)
def test_a_line_naming_no_tier_carries_absence_not_an_empty_string(pricing: Mapping[str, Any] | None) -> None:
    assert price_details(dict(SUBSCRIPTION_LINE, pricing=pricing)) == PriceRef(None, None), (
        f"should be absent: {pricing!r}"
    )


def test_the_projection_digest_covers_every_field_a_line_row_carries() -> None:
    """`_EMPTY_LINE` is what the digest is taken from, and what an invoice's own row
    fills in for the fields it has no line for. A column added to `shape_line` and
    not to it would leave the digest unmoved — so the re-chain that column needs
    would not happen — and would leave that column off the invoice's own row.

    `invoice_key` is the one field outside the set on purpose: `line_row` pops it
    before the row is emitted, so it reaches neither bronze nor the digest.
    """
    shaped = set(shape_line(SUBSCRIPTION_LINE, "in_1ABC")) - {"invoice_key"}

    assert shaped == set(chain_module._EMPTY_LINE), "one of the two declarations gained a field the other lacks"


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


@pytest.mark.parametrize("value", [2500.0, "2500", True, [], {}])
def test_a_unit_amount_of_another_type_is_reported_as_unreadable(value: object) -> None:
    """Absence and a changed type look the same downstream; only one is a fault."""
    lines = [dict(SUBSCRIPTION_LINE, hosted_invoice_unit_amount=value)]
    assert unreadable_seat_prices(lines) == [type(value).__name__]


def test_an_unpriced_seat_line_is_absence_not_an_unreadable_type() -> None:
    """The vendor leaving a subscription line unpriced is a state we report."""
    assert unreadable_seat_prices([dict(SUBSCRIPTION_LINE, hosted_invoice_unit_amount=None)]) == []
    assert unreadable_seat_prices([SUBSCRIPTION_LINE]) == []


def test_only_seat_pricing_lines_are_judged_on_their_unit_amount() -> None:
    """Extra usage and prorations never state a seat price, so their type is not ours."""
    assert unreadable_seat_prices([dict(EXTRA_USAGE_LINE, hosted_invoice_unit_amount="2000")]) == []
    assert unreadable_seat_prices([dict(PRORATION_CREDIT, hosted_invoice_unit_amount="1500")]) == []


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
