"""Pure logic of the Stripe hosted-invoice chain — no network, no CDK.

The claude.ai wrapper (`GET /api/stripe/{org}/invoices`) carries neither an
invoice id nor line items, only a `hosted_invoice_url`. Everything that gives an
invoice its meaning — the per-seat price, the tier, whether a line is a
subscription or extra usage — lives behind that URL. This module holds the
parsing and classification that reaching it requires, kept free of I/O so the
rules are exercised without a network call.
"""

from __future__ import annotations

import logging
import re
from collections.abc import Callable, Iterator, Mapping, Sequence
from dataclasses import dataclass
from typing import Any

logger = logging.getLogger("airbyte")

# `https://invoice.stripe.com/i/{acct}/{token}?s=ap`. The token is the raw path
# segment used for the bootstrap request, not a decoded identifier. The host is
# part of the pattern on purpose: these two segments are interpolated into a
# request URL, so a link pointing anywhere else must not contribute them.
_HOSTED_URL = re.compile(
    r"^https://invoice\.stripe\.com/i/(acct_[A-Za-z0-9_-]+)/((?:test|live)_[A-Za-z0-9_-]+)(?:[?#]|$)"
)

CATEGORY_SUBSCRIPTIONS = "subscriptions"
CATEGORY_OVERUSAGE = "overusage"


class StripeChainError(Exception):
    """A step of the chain did not return what the next step needs."""


@dataclass(frozen=True)
class HostedRef:
    """The account and token pair a `hosted_invoice_url` resolves to."""

    acct: str
    token: str


def parse_hosted_invoice_url(url: str | None) -> HostedRef | None:
    """Split a hosted invoice URL into its account and token.

    Returns None both for an absent URL and for one that does not match. The
    caller keeps those apart: only a URL the vendor did offer, and which stopped
    matching, says anything about the format having changed.
    """
    if not url:
        return None
    match = _HOSTED_URL.match(url)
    return HostedRef(match.group(1), match.group(2)) if match else None


def classify_line(line: Mapping[str, Any]) -> str:
    """Categorise one invoice line by its Stripe parent, never by its text.

    A subscription-item parent is the recurring seat charge; an invoice-item
    parent is a one-off, which for this vendor is prepaid extra usage.
    Descriptions are localised marketing copy and change without notice.
    """
    parent = line.get("parent") or {}
    if parent.get("subscription_item_details") is not None:
        return CATEGORY_SUBSCRIPTIONS
    if parent.get("invoice_item_details") is not None:
        return CATEGORY_OVERUSAGE
    return CATEGORY_OVERUSAGE


def is_proration(line: Mapping[str, Any]) -> bool:
    """Whether the line is a mid-period adjustment rather than a period's charge.

    A seat-count change emits a credit for the unused time and a charge for the
    remainder. Both are real money and both are `subscriptions`, but neither
    carries a unit price, and their amounts are partial-period — dividing one by
    its quantity yields a number that is not a seat price.
    """
    parent = line.get("parent") or {}
    details = parent.get("subscription_item_details") or {}
    return bool(details.get("proration"))


def seat_unit_amount(line: Mapping[str, Any]) -> int | None:
    """The per-seat price on a line, in minor units, or None when it has none.

    Only a non-proration subscription line prices a seat. Everything else —
    extra usage, prorations, a subscription line the vendor left unpriced —
    yields None, which downstream renders as absence rather than as zero.
    """
    if classify_line(line) != CATEGORY_SUBSCRIPTIONS or is_proration(line):
        return None
    amount = line.get("hosted_invoice_unit_amount")
    # `bool` is an `int` in Python, and a boolean here would price a seat at 1.
    if isinstance(amount, bool) or not isinstance(amount, int):
        return None
    return int(amount)


def shape_line(line: Mapping[str, Any], invoice_key: str) -> dict[str, Any]:
    """Project one Stripe line onto the columns bronze keeps.

    `period` dates the line to the window it charges for, which is not always
    the window the invoice was issued in.
    """
    period = line.get("period") or {}
    return {
        "invoice_key": invoice_key,
        "line_id": line.get("id"),
        "description": line.get("description"),
        "product_name": line.get("hosted_invoice_product_name"),
        "tier_label": line.get("hosted_invoice_tier_label"),
        "category": classify_line(line),
        "is_proration": is_proration(line),
        "amount": line.get("amount"),
        "currency": line.get("currency"),
        "quantity": line.get("quantity"),
        "unit_amount": line.get("hosted_invoice_unit_amount"),
        "seat_unit_amount": seat_unit_amount(line),
        "period_start_ts": period.get("start"),
        "period_end_ts": period.get("end"),
    }


def read_bootstrap(payload: Mapping[str, Any]) -> tuple[str, str]:
    """Take the invoice id and ephemeral key out of the bootstrap response.

    The key authorises the next two requests and is never persisted or logged.
    The id is shape-checked because it is interpolated into a URL.
    """
    invoice_id = payload.get("invoice_id")
    ephemeral_key = payload.get("ephemeral_key")
    if not invoice_id or not ephemeral_key:
        raise StripeChainError("bootstrap response carried no invoice_id/ephemeral_key")
    if not re.fullmatch(r"in_[A-Za-z0-9_-]+", str(invoice_id)):
        raise StripeChainError(f"bootstrap returned an unexpected invoice id shape: {invoice_id!r}")
    return str(invoice_id), str(ephemeral_key)


# A parse failure on one invoice is one unpriced row; a parse failure on nearly
# all of them means the URL format changed, and a full run of unpriced rows
# would read as "the vendor stopped charging for seats".
DRIFT_RATIO = 0.5

CHAIN_OK = "ok"
CHAIN_FAILED = "failed"
CHAIN_UNPARSABLE = "unparsable_url"
CHAIN_NO_URL = "no_hosted_url"

# The line-shaped fields, all absent, for an invoice's own row. Keeping the
# field set identical keeps bronze rectangular.
_EMPTY_LINE: Mapping[str, Any] = {
    "line_id": None,
    "description": None,
    "product_name": None,
    "tier_label": None,
    "category": None,
    "is_proration": None,
    "amount": None,
    "currency": None,
    "quantity": None,
    "unit_amount": None,
    "seat_unit_amount": None,
    "period_start_ts": None,
    "period_end_ts": None,
}


class UrlFormatDrift(RuntimeError):
    """Too few hosted URLs parsed for the run to be trustworthy."""


def invoice_identity(invoice: Mapping[str, Any]) -> dict[str, Any]:
    """The invoice facts a line needs to place itself, carried on every row."""
    return {
        "invoice_status": invoice.get("status"),
        "invoice_created_ts": invoice.get("created_ts"),
        "invoice_currency": invoice.get("currency"),
    }


def invoice_ledger(invoice: Mapping[str, Any]) -> dict[str, Any]:
    """The invoice's money and the wrapper's own fields, for its row alone.

    `total_excluding_tax` is the net figure the class contract wants; `total`
    sits beside it so the tax component stays visible rather than dropped.
    """
    return {
        "invoice_due_date_ts": invoice.get("due_date_ts"),
        "invoice_total": invoice.get("total"),
        "invoice_total_excluding_tax": invoice.get("total_excluding_tax"),
        # `num_seats` may be absent; where present it names one line's quantity
        # while an invoice can price several tiers. Kept for provenance; the seat
        # count comes from the line's own quantity.
        "invoice_num_seats": invoice.get("num_seats"),
        "invoice_payment_intent": invoice.get("payment_intent"),
    }


# The invoice-level fields, all absent, for a line's row: an invoice's money
# belongs to exactly one row, so summing it needs no dedup.
_EMPTY_LEDGER: Mapping[str, Any] = {
    "invoice_due_date_ts": None,
    "invoice_total": None,
    "invoice_total_excluding_tax": None,
    "invoice_num_seats": None,
    "invoice_payment_intent": None,
}


def lines_period(lines: Sequence[Mapping[str, Any]]) -> tuple[int | None, int | None]:
    """The window a whole invoice charges for: the span its lines cover."""
    periods = [line.get("period") or {} for line in lines]
    starts = [period["start"] for period in periods if period.get("start")]
    ends = [period["end"] for period in periods if period.get("end")]
    return (min(starts) if starts else None, max(ends) if ends else None)


def invoice_row(
    invoice: Mapping[str, Any],
    chain_status: str,
    invoice_id: str | None,
    period: tuple[int | None, int | None] = (None, None),
) -> dict[str, Any]:
    """The invoice's own row: its money, how far its chain got, and no line.

    Dated by the window its lines charge for, exactly as the lines themselves are:
    a monthly invoice is raised at the period boundary, so dating it by its own
    creation would file its money in the neighbouring month. An invoice with no
    lines has nothing but that creation day to fall back on.
    """
    row = dict(
        invoice_identity(invoice),
        **invoice_ledger(invoice),
        **_EMPTY_LINE,
        chain_status=chain_status,
        invoice_id=invoice_id,
    )
    row["period_start_ts"], row["period_end_ts"] = period
    return row


def line_row(invoice: Mapping[str, Any], invoice_id: str, line: Mapping[str, Any]) -> dict[str, Any]:
    """One line's own row, carrying the line's money and none of the invoice's."""
    shaped = shape_line(line, invoice_id)
    shaped.pop("invoice_key", None)
    return dict(invoice_identity(invoice), **_EMPTY_LEDGER, chain_status=CHAIN_OK, invoice_id=invoice_id, **shaped)


def build_records(
    invoices: Sequence[Mapping[str, Any]],
    fetch_lines: Callable[[str, str], tuple[str, Sequence[Mapping[str, Any]]]],
    *,
    drift_ratio: float = DRIFT_RATIO,
) -> Iterator[dict[str, Any]]:
    """Turn wrapper invoices into records, degrading per invoice.

    `fetch_lines(acct, token)` performs the Stripe hops and returns the invoice
    id with its full line set; everything about *when* to call it, and what to
    emit when it fails, lives here so those rules are testable without a socket.

    Every invoice yields its own row carrying its money and how far its chain
    got; a chain that returned adds one row per line on top. So an invoice's
    money sits on exactly one row whatever happens, and a run that enriches an
    invoice replaces the row an earlier unenriched run wrote for it.

    Four outcomes per invoice, and one for the run:
      * the wrapper offered no URL -> `no_hosted_url`
      * the URL did not parse      -> `unparsable_url`
      * the chain raised           -> `failed`
      * the chain returned         -> `ok`, plus one row per line
    and if more than `drift_ratio` of the URLs the vendor did offer failed to
    parse, the run raises rather than writing rows that carry money but no prices.
    """
    parsed = [(inv, parse_hosted_invoice_url(inv.get("hosted_invoice_url"))) for inv in invoices]

    offered = [inv for inv, _ in parsed if inv.get("hosted_invoice_url")]
    malformed = sum(1 for inv, ref in parsed if ref is None and inv.get("hosted_invoice_url"))
    if offered and malformed > len(offered) * drift_ratio:
        raise UrlFormatDrift(
            f"{malformed} of {len(offered)} hosted invoice URLs did not parse - the format "
            "has almost certainly changed; refusing to write a run of unpriced rows"
        )

    for invoice, ref in parsed:
        if ref is None:
            absent = not invoice.get("hosted_invoice_url")
            yield invoice_row(invoice, CHAIN_NO_URL if absent else CHAIN_UNPARSABLE, None)
            continue
        try:
            invoice_id, lines = fetch_lines(ref.acct, ref.token)
        except Exception as error:  # noqa: BLE001 - one invoice must not end the run
            # A gap in pricing, not a reason to lose the invoice or fail the sync.
            # The type and status only: a request error stringifies its URL, and
            # the hosted-invoice hop carries the token in that URL.
            logger.warning(
                "stripe chain failed for the invoice created at %s: %s (HTTP %s)",
                invoice.get("created_ts"),
                type(error).__name__,
                getattr(getattr(error, "response", None), "status_code", "n/a"),
            )
            yield invoice_row(invoice, CHAIN_FAILED, None)
            continue

        yield invoice_row(invoice, CHAIN_OK, invoice_id, lines_period(lines))
        for line in lines:
            yield line_row(invoice, invoice_id, line)


def unique_key_parts(record: Mapping[str, Any]) -> tuple[Any, ...]:
    """The natural key of a record, which differs by what the row carries.

    A line is identified by Stripe's own ids. An invoice's own row has no line
    id, and no invoice id at all until its chain completes, so it keys on what
    the wrapper reports on every run — which is what lets an enriched run replace
    the row an unenriched one wrote instead of adding a second one beside it.
    The chain outcome is deliberately NOT part of that key: an invoice that fails
    one way and then another must stay one row. The amount and due date join it
    because a payment intent is absent on some invoices, and creation timestamps
    collide across a batch issued at once — two invoices sharing one key would
    leave only one row.
    """
    if record.get("line_id"):
        return (record.get("invoice_id"), record.get("line_id"))
    return (
        "invoice",
        record.get("invoice_created_ts"),
        record.get("invoice_payment_intent") or "",
        record.get("invoice_total"),
        record.get("invoice_due_date_ts"),
    )
