"""The invoice stream: claude.ai wrapper, then the Stripe hosted chain per invoice.

One stream, one bronze table, two kinds of record: every invoice emits its own
row carrying its money and how far its chain got, and a chain that completed adds
one row per line beside it. An invoice's ledger therefore survives a broken chain
without a fabricated price, and it sits on exactly one row — so a later run that
enriches the invoice replaces that row rather than adding its money a second time.

The chain is four hops per invoice:

    claude.ai wrapper   -> hosted_invoice_url
    invoice.stripe.com  -> (acct, token)          [parsed, not requested]
    invoicedata.stripe  -> invoice_id + ephemeral key
    api.stripe.com      -> /lines

The ephemeral key authorises the last hop, is short-lived, and is never written
to a record, a state message or a log line — which is why the chain lives inside
one stream instead of a parent/child pair, whose parent records would persist it.

A `hosted_invoice_url` is not durable: Stripe expires it 30 days after the
invoice's due date, and never later than 120 days, after which it stops
resolving. The wrapper re-issues a fresh URL on every list call, which is why
invoices from any month still enrich — so the URL is followed inside the run
that fetched it and is never stored to be followed later.

This module is the I/O shell. What to emit, and what to emit when a hop fails,
lives in `stripe_chain.build_records` so those rules are exercised without a
socket.
"""

from __future__ import annotations

import time
from collections.abc import Iterable, Mapping, MutableMapping, Sequence
from datetime import UTC, datetime
from typing import Any

import requests
from airbyte_cdk.sources.streams import Stream

from source_claude_team_invoices.stripe_chain import StripeChainError, build_records, read_bootstrap, unique_key_parts

STRIPE_VERSION = "2026-06-24.dahlia"
BOOTSTRAP_HOST = "https://invoicedata.stripe.com"
STRIPE_API = "https://api.stripe.com/v1"

PER_PAGE = 12
# Bounds on the two cursor walks, far above any real history. They guard
# against a cursor the endpoint ignores, which would otherwise loop forever.
MAX_PAGES = 50
MAX_LINE_PAGES = 20
TIMEOUT = 30
# Paced between invoices, not inside one: the bootstrap host is undocumented and
# a run walks every invoice, so the chain is the only part whose rate we choose.
CHAIN_DELAY_SECS = 0.25


def _now_iso() -> str:
    return datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")


class InvoiceLines(Stream):
    """`claude_team_invoice_lines` — one record per invoice, plus one per line."""

    primary_key = "unique_key"

    def __init__(self, config: Mapping[str, Any]) -> None:
        self._proxy_url = str(config["proxy_url"]).rstrip("/")
        self._proxy_token = config["proxy_auth_token"]
        self._org_id = config["claude_org_id"]
        self._tenant_id = config["insight_tenant_id"]
        self._source_id = config["insight_source_id"]
        self._session = requests.Session()
        self._chained = False

    @property
    def name(self) -> str:
        return "claude_team_invoice_lines"

    def get_json_schema(self) -> Mapping[str, Any]:
        from source_claude_team_invoices.source import load_stream_schema

        return load_stream_schema(self.name)

    def _get_json(self, url: str, headers: Mapping[str, str] | None = None) -> Any:
        response = self._session.get(url, headers=dict(headers or {}), timeout=TIMEOUT)
        response.raise_for_status()
        return response.json()

    def _walk_invoices(self) -> list[Mapping[str, Any]]:
        """Page the claude.ai wrapper until it stops offering a cursor."""
        invoices: list[Mapping[str, Any]] = []
        cursor: str | None = None
        for _ in range(MAX_PAGES):
            url = f"{self._proxy_url}/api/stripe/{self._org_id}/invoices?limit={PER_PAGE}&page={cursor or ''}"
            body = self._get_json(url, {"Authorization": f"Bearer {self._proxy_token}"})
            invoices.extend(body.get("invoices") or [])
            cursor = body.get("next_page")
            if not cursor:
                return invoices
        # Stopping silently here would drop the oldest invoices and read as a
        # shorter billing history rather than as a fault.
        raise RuntimeError(
            f"invoice pagination did not terminate within {MAX_PAGES} pages "
            f"({MAX_PAGES * PER_PAGE} invoices); the page cursor is not advancing"
        )

    def _fetch_lines(self, acct: str, token: str) -> tuple[str, Sequence[Mapping[str, Any]]]:
        """Run the Stripe hops and return the invoice id with its full line set."""
        if self._chained:
            time.sleep(CHAIN_DELAY_SECS)
        self._chained = True

        bootstrap = self._get_json(f"{BOOTSTRAP_HOST}/hosted_invoice_page/{acct}/{token}")
        invoice_id, ephemeral_key = read_bootstrap(bootstrap)

        headers = {"Authorization": f"Bearer {ephemeral_key}", "Stripe-Version": STRIPE_VERSION, "Stripe-Account": acct}
        # Issued back to back: the key is short-lived, so nothing sleeps between hops.
        lines: list[Mapping[str, Any]] = []
        starting_after: str | None = None
        for _ in range(MAX_LINE_PAGES):
            url = f"{STRIPE_API}/invoices/{invoice_id}/lines?limit=100"
            if starting_after:
                url += f"&starting_after={starting_after}"
            page = self._get_json(url, headers)
            batch = page.get("data") or []
            lines.extend(batch)
            if not page.get("has_more"):
                return invoice_id, lines
            if not batch or not batch[-1].get("id"):
                raise StripeChainError("line page claims has_more with no cursor to advance")
            starting_after = batch[-1]["id"]
        raise StripeChainError(f"line pagination exceeded {MAX_LINE_PAGES} pages")

    def _envelope(self, record: dict[str, Any]) -> dict[str, Any]:
        """Inject the framework fields every bronze row carries (ADR-0004)."""
        record["tenant_id"] = self._tenant_id
        record["source_id"] = self._source_id
        record["data_source"] = "insight_claude_team"
        record["collected_at"] = _now_iso()
        record["unique_key"] = "-".join([self._tenant_id, self._source_id, *(str(p) for p in unique_key_parts(record))])
        return record

    def read_records(self, sync_mode: Any, **kwargs: Any) -> Iterable[MutableMapping[str, Any]]:
        for record in build_records(self._walk_invoices(), self._fetch_lines):
            yield self._envelope(record)
