"""The shapes this connector reads out of the wrapper, Stripe, its config and its state.

Each model declares only the fields the connector reads; anything else in a body
is ignored.
"""

from __future__ import annotations

from collections.abc import Mapping
from typing import Any

from pydantic import BaseModel, ConfigDict, Field, JsonValue, StrictFloat, StrictInt, ValidationError

# Strict: these values are emitted, so a lax coercion would rewrite what the vendor sent.
Number = StrictInt | StrictFloat


class _Frozen(BaseModel):
    model_config = ConfigDict(frozen=True)


def validation_summary(error: ValidationError) -> str:
    """Where and how a body failed to parse, without the offending values.

    INVARIANT: never the input — a body can carry personal data or a credential.
    """
    return "; ".join(
        f"{'.'.join(str(part) for part in issue['loc']) or '<root>'}: {issue['type']}"
        for issue in error.errors(include_input=False)
    )


class ConnectorConfig(_Frozen):
    proxy_url: str
    proxy_auth_token: str
    claude_org_id: str
    insight_tenant_id: str
    insight_source_id: str


def parse_config(config: Mapping[str, Any]) -> ConnectorConfig:
    try:
        return ConnectorConfig.model_validate(config)
    except ValidationError as error:
        raise ValueError(f"connector config is invalid: {validation_summary(error)}") from None


class Invoice(_Frozen):
    """One invoice as the claude.ai wrapper lists it: money and a hosted URL, no id.

    INVARIANT: a field the connector only copies into bronze is `JsonValue`, so a
    changed type reaches bronze as sent instead of failing the parse.
    """

    status: str | None = None
    hosted_invoice_url: str | None = None
    created_ts: JsonValue = None
    due_date_ts: JsonValue = None
    currency: JsonValue = None
    total: JsonValue = None
    total_excluding_tax: JsonValue = None
    num_seats: JsonValue = None
    payment_intent: JsonValue = None


class InvoicePage(_Frozen):
    invoices: list[Invoice] | None = None
    # The wrapper's cursor is opaque and only echoed back into the next URL.
    next_page: str | int | None = None


class Bootstrap(_Frozen):
    """The hosted-invoice bootstrap answer. The id is interpolated into a URL."""

    invoice_id: str = Field(pattern=r"^in_[A-Za-z0-9_-]+$")
    ephemeral_key: str = Field(min_length=1)


class SubscriptionItemDetails(_Frozen):
    proration: bool | None = None


class LineParent(_Frozen):
    subscription_item_details: SubscriptionItemDetails | None = None


class PriceDetails(_Frozen):
    price: str | None = None
    product: str | None = None


class LinePricing(_Frozen):
    price_details: PriceDetails | None = None


class LinePeriod(_Frozen):
    start: Number | None = None
    end: Number | None = None


class InvoiceLine(_Frozen):
    """One Stripe invoice line.

    INVARIANT: a field the connector only copies into bronze is `JsonValue`, so a
    changed type reaches bronze as sent instead of failing the chain.
    `hosted_invoice_unit_amount` is one too: `unreadable_seat_prices` judges its
    type per line, and only on a line that prices a seat.
    """

    id: str | None = None
    description: JsonValue = None
    amount: JsonValue = None
    currency: JsonValue = None
    quantity: JsonValue = None
    hosted_invoice_unit_amount: JsonValue = None
    hosted_invoice_product_name: JsonValue = None
    hosted_invoice_tier_label: JsonValue = None
    parent: LineParent | None = None
    pricing: LinePricing | None = None
    period: LinePeriod | None = None


class LinePage(_Frozen):
    data: list[InvoiceLine] | None = None
    has_more: bool | None = None


class EnrichedInvoice(_Frozen):
    """What an earlier chain found for an invoice: the keys its bronze rows carry."""

    invoice_id: str
    period_start_ts: Number | None = None
    period_end_ts: Number | None = None


class StreamState(_Frozen):
    """Entries are read one by one, so one unreadable entry costs one re-chain."""

    enriched: dict[str, JsonValue] | None = None
    line_projection: str | None = None
