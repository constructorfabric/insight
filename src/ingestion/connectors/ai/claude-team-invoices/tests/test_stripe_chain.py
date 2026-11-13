"""Rules of the Stripe chain, exercised without a network call.

Fixtures carry the shapes the vendor emits, with synthetic values: a monthly
subscription line, a proration credit from a mid-period seat change, and a
prepaid extra-usage purchase.
"""

import base64
import json

import pytest
from pydantic import JsonValue
from source_claude_team_invoices import stripe_chain as chain_module
from source_claude_team_invoices.models import (
    Bootstrap,
    InvoiceLine,
    LinePage,
    LineParent,
    LinePeriod,
    LinePricing,
    PriceDetails,
    SubscriptionItemDetails,
)
from source_claude_team_invoices.stripe_chain import (
    CATEGORY_OVERUSAGE,
    CATEGORY_SUBSCRIPTIONS,
    PriceRef,
    StripeChainError,
    classify_line,
    is_proration,
    parse_hop,
    parse_hosted_invoice_url,
    price_details,
    seat_unit_amount,
    shape_line,
    stable_invoice_ref,
    unreadable_seat_prices,
)

SUBSCRIPTION_LINE = InvoiceLine(
    id="il_standard",
    description="3 x Example plan - Standard (at $10.00 / month)",
    amount=3000,
    currency="usd",
    quantity=3,
    hosted_invoice_unit_amount=1000,
    hosted_invoice_product_name="Example plan - Standard",
    hosted_invoice_tier_label="Standard",
    parent=LineParent(subscription_item_details=SubscriptionItemDetails(proration=False)),
    period=LinePeriod(start=1754092800, end=1756771200),
)

PRORATION_CREDIT = InvoiceLine(
    id="il_unused",
    description="Unused time on 3 x Example plan - Standard",
    amount=-1500,
    quantity=3,
    hosted_invoice_unit_amount=None,
    parent=LineParent(subscription_item_details=SubscriptionItemDetails(proration=True)),
)

# An invoice-item parent; the model keeps no field of it, since only a
# subscription-item parent changes the category.
EXTRA_USAGE_LINE = InvoiceLine(
    id="il_prepaid",
    description="Prepaid extra usage, Example plan",
    amount=2000,
    quantity=1,
    hosted_invoice_unit_amount=2000,
    parent=LineParent(),
)


def with_unit_amount(line: InvoiceLine, amount: JsonValue) -> InvoiceLine:
    return line.model_copy(update={"hosted_invoice_unit_amount": amount})


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
    pricing = LinePricing(price_details=PriceDetails(price="price_1ABC", product="prod_1ABC"))
    line = SUBSCRIPTION_LINE.model_copy(update={"pricing": pricing})
    assert price_details(line) == PriceRef("price_1ABC", "prod_1ABC")
    shaped = shape_line(line, "in_1ABC")
    assert (shaped["price_id"], shaped["product_id"]) == ("price_1ABC", "prod_1ABC")


@pytest.mark.parametrize(
    "pricing",
    [
        None,
        LinePricing(),
        LinePricing(price_details=PriceDetails()),
        LinePricing(price_details=PriceDetails(price="", product="")),
    ],
)
def test_a_line_naming_no_tier_carries_absence_not_an_empty_string(pricing: LinePricing | None) -> None:
    assert price_details(SUBSCRIPTION_LINE.model_copy(update={"pricing": pricing})) == PriceRef(None, None), (
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
    disguised = EXTRA_USAGE_LINE.model_copy(update={"description": "3 x Example plan - Standard"})
    assert classify_line(disguised) == CATEGORY_OVERUSAGE


def test_unknown_parent_falls_back_to_overusage() -> None:
    assert classify_line(InvoiceLine(parent=LineParent())) == CATEGORY_OVERUSAGE
    assert classify_line(InvoiceLine()) == CATEGORY_OVERUSAGE


def test_only_a_settled_subscription_line_prices_a_seat() -> None:
    assert seat_unit_amount(SUBSCRIPTION_LINE) == 1000
    assert seat_unit_amount(PRORATION_CREDIT) is None, "a proration carries no seat price"
    assert seat_unit_amount(EXTRA_USAGE_LINE) is None, "extra usage is not a seat"


def test_a_subscription_line_the_vendor_left_unpriced_yields_absence() -> None:
    assert seat_unit_amount(with_unit_amount(SUBSCRIPTION_LINE, None)) is None


@pytest.mark.parametrize("value", [True, False])
def test_a_boolean_unit_amount_is_absence_not_a_price(value: bool) -> None:
    """`bool` is an `int` in Python, so an unguarded check prices a seat at 1."""
    assert seat_unit_amount(with_unit_amount(SUBSCRIPTION_LINE, value)) is None


@pytest.mark.parametrize("value", [2500.0, "2500", True, [], {}])
def test_a_unit_amount_of_another_type_is_reported_as_unreadable(value: JsonValue) -> None:
    """Absence and a changed type look the same downstream; only one is a fault."""
    lines = [with_unit_amount(SUBSCRIPTION_LINE, value)]
    assert unreadable_seat_prices(lines) == [type(value).__name__]


def test_an_unpriced_seat_line_is_absence_not_an_unreadable_type() -> None:
    """The vendor leaving a subscription line unpriced is a state we report."""
    assert unreadable_seat_prices([with_unit_amount(SUBSCRIPTION_LINE, None)]) == []
    assert unreadable_seat_prices([SUBSCRIPTION_LINE]) == []


def test_only_seat_pricing_lines_are_judged_on_their_unit_amount() -> None:
    """Extra usage and prorations never state a seat price, so their type is not ours."""
    assert unreadable_seat_prices([with_unit_amount(EXTRA_USAGE_LINE, "2000")]) == []
    assert unreadable_seat_prices([with_unit_amount(PRORATION_CREDIT, "1500")]) == []


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
    body = Bootstrap(invoice_id="in_1ABC", ephemeral_key="ek_x").model_dump_json()
    bootstrap = parse_hop(Bootstrap, body, "bootstrap")
    assert (bootstrap.invoice_id, bootstrap.ephemeral_key) == ("in_1ABC", "ek_x")


@pytest.mark.parametrize(
    "body",
    [
        "{}",
        '{"invoice_id": "in_1ABC"}',
        '{"ephemeral_key": "ek_x"}',
        '{"invoice_id": "", "ephemeral_key": "ek_x"}',
        '{"invoice_id": "in_1ABC", "ephemeral_key": ""}',
        '{"invoice_id": 7, "ephemeral_key": "ek_x"}',
        "not json",
    ],
)
def test_incomplete_bootstrap_fails_loudly_without_quoting_the_key(body: str) -> None:
    with pytest.raises(StripeChainError, match="bootstrap answered an unexpected shape") as raised:
        parse_hop(Bootstrap, body, "bootstrap")
    assert "ek_x" not in str(raised.value), f"should not quote the key: {body!r}"


@pytest.mark.parametrize("invoice_id", ["../etc", "sub_1ABC", "in_ABC/../x", "in_ABC?x=1", "in_ABC\n"])
def test_bootstrap_rejects_an_invoice_id_that_is_not_one(invoice_id: str) -> None:
    body = json.dumps({"invoice_id": invoice_id, "ephemeral_key": "ek_x"})
    with pytest.raises(StripeChainError) as raised:
        parse_hop(Bootstrap, body, "bootstrap")
    assert "invoice_id: string_pattern_mismatch" in str(raised.value), f"should reject: {invoice_id!r}"
    assert invoice_id not in str(raised.value)


def test_a_line_page_of_the_wrong_shape_names_the_field_and_not_its_value() -> None:
    body = '{"data": [{"id": "il_1", "parent": {"subscription_item_details": "secret-text"}}], "has_more": false}'
    with pytest.raises(StripeChainError) as raised:
        parse_hop(LinePage, body, "line page")
    assert "data.0.parent.subscription_item_details: model_type" in str(raised.value)
    assert "secret-text" not in str(raised.value)


@pytest.mark.parametrize("start", ['"1754092800"', "true"])
def test_a_period_that_is_not_a_number_fails_the_chain_rather_than_being_rewritten(start: str) -> None:
    """A coerced value would reach bronze and state as something the vendor never sent."""
    with pytest.raises(StripeChainError, match="data.0.period.start") as raised:
        parse_hop(LinePage, f'{{"data": [{{"period": {{"start": {start}}}}}]}}', "line page")
    assert "1754092800" not in str(raised.value), f"should not quote: {start!r}"


def test_a_line_page_carries_the_unit_amount_as_the_vendor_sent_it() -> None:
    """Its type is judged per line by `unreadable_seat_prices`, not by the parse."""
    page = parse_hop(LinePage, '{"data": [{"hosted_invoice_unit_amount": "2000"}]}', "line page")
    assert page.data is not None
    assert page.data[0].hosted_invoice_unit_amount == "2000"
