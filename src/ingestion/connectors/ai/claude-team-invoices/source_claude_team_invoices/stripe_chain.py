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
from dataclasses import dataclass
from typing import Any, Callable, Iterator, Mapping, Optional, Sequence

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


def parse_hosted_invoice_url(url: Optional[str]) -> Optional[HostedRef]:
    """Split a hosted invoice URL into its account and token.

    Returns None when the URL does not match. A single miss is one unenriched
    invoice; a miss on every invoice means the format changed, which the caller
    treats as a failure rather than writing rows without prices.
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


def seat_unit_amount(line: Mapping[str, Any]) -> Optional[int]:
    """The per-seat price on a line, in minor units, or None when it has none.

    Only a non-proration subscription line prices a seat. Everything else —
    extra usage, prorations, a subscription line the vendor left unpriced —
    yields None, which downstream renders as absence rather than as zero.
    """
    if classify_line(line) != CATEGORY_SUBSCRIPTIONS or is_proration(line):
        return None
    amount = line.get("hosted_invoice_unit_amount")
    return int(amount) if isinstance(amount, int) else None


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

# The line-shaped fields, all absent, for a row that carries an invoice without
# any line. Keeping the field set identical keeps bronze rectangular.
_EMPTY_LINE: Mapping[str, Any] = {
    "invoice_id": None,
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


def invoice_fields(invoice: Mapping[str, Any]) -> dict[str, Any]:
    """Invoice-level facts carried on every line of that invoice.

    `total_excluding_tax` is the net figure the class contract wants; `total`
    sits beside it so the tax component stays visible rather than dropped.
    """
    return {
        "invoice_status": invoice.get("status"),
        "invoice_created_ts": invoice.get("created_ts"),
        "invoice_due_date_ts": invoice.get("due_date_ts"),
        "invoice_currency": invoice.get("currency"),
        "invoice_total": invoice.get("total"),
        "invoice_total_excluding_tax": invoice.get("total_excluding_tax"),
        # Reported on a minority of invoices, and where it is reported it names
        # one line's quantity while the invoice can cover several tiers. Kept
        # for provenance; the seat count comes from the line's own quantity.
        "invoice_num_seats": invoice.get("num_seats"),
        "invoice_payment_intent": invoice.get("payment_intent"),
    }


def build_records(
    invoices: Sequence[Mapping[str, Any]],
    fetch_lines: Callable[[str, str], tuple[str, Sequence[Mapping[str, Any]]]],
    *,
    drift_ratio: float = DRIFT_RATIO,
) -> Iterator[dict[str, Any]]:
    """Turn wrapper invoices into line records, degrading per invoice.

    `fetch_lines(acct, token)` performs the Stripe hops and returns the invoice
    id with its full line set; everything about *when* to call it, and what to
    emit when it fails, lives here so those rules are testable without a socket.

    Three outcomes per invoice, and one for the run:
      * the URL did not parse   -> one row, `unparsable_url`, no line
      * the chain raised        -> one row, `failed`, no line
      * the chain returned      -> one row per line, `ok`
    and if more than `drift_ratio` of the URLs did not parse, the run raises
    rather than writing a set of rows that carry money but no prices.
    """
    parsed = [(inv, parse_hosted_invoice_url(inv.get("hosted_invoice_url"))) for inv in invoices]

    unparsable = sum(1 for _, ref in parsed if ref is None)
    if invoices and unparsable > len(invoices) * drift_ratio:
        raise UrlFormatDrift(
            f"{unparsable} of {len(invoices)} hosted invoice URLs did not parse - the format "
            "has almost certainly changed; refusing to write a run of unpriced rows"
        )

    for invoice, ref in parsed:
        common = invoice_fields(invoice)
        if ref is None:
            yield dict(common, chain_status=CHAIN_UNPARSABLE, **_EMPTY_LINE)
            continue
        try:
            invoice_id, lines = fetch_lines(ref.acct, ref.token)
        except Exception as error:  # noqa: BLE001 - one invoice must not end the run
            # A gap in pricing, not a reason to lose the invoice or fail the sync.
            logger.warning(
                "stripe chain failed for the invoice created at %s: %s",
                invoice.get("created_ts"),
                error,
            )
            yield dict(common, chain_status=CHAIN_FAILED, **_EMPTY_LINE)
            continue

        for line in lines:
            shaped = shape_line(line, invoice_id)
            shaped.pop("invoice_key", None)
            shaped["invoice_id"] = invoice_id
            yield dict(common, chain_status=CHAIN_OK, **shaped)


def unique_key_parts(record: Mapping[str, Any]) -> tuple[Any, ...]:
    """The natural key of a record, which differs by how far its chain got.

    An enriched line is identified by Stripe's own ids. A row that carries only
    an invoice has neither, so it falls back to what the wrapper gave us.
    """
    if record.get("chain_status") == CHAIN_OK:
        return (record.get("invoice_id"), record.get("line_id") or "")
    return (
        record.get("chain_status"),
        record.get("invoice_created_ts"),
        record.get("invoice_payment_intent") or "",
    )
