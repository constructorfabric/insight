# Claude Team invoices

The invoiced layer for Claude Team: what Anthropic actually billed, and the only
place a seat's price appears.

## Why a CDK connector

The invoice list alone is declarative-shaped. Its line items are not: the
claude.ai wrapper carries no invoice id and no lines, only a
`hosted_invoice_url`, and reaching the lines takes a three-hop chain across two
hosts where each hop's credential comes out of the previous hop's response.
Same reasoning as github-copilot ADR-0001.

## The chain

| Step | Request | What it yields |
|---|---|---|
| 1 | `GET {proxy}/api/stripe/{org}/invoices` | the wrapper's rows: `total`, `total_excluding_tax`, `currency`, `status`, `num_seats`, `hosted_invoice_url` — no invoice id, no lines |
| 2 | parse `https://invoice.stripe.com/i/{acct}/{token}?s=ap` | the account and token identifiers |
| 3 | `GET invoicedata.stripe.com/hosted_invoice_page/{acct}/{token}` | `invoice_id` and a short-lived `ephemeral_key` |
| 4 | `GET api.stripe.com/v1/invoices/{id}/lines?limit=100`, paginated on `has_more` | the full line set |

Step 3 carries no header: the token inside its URL is what authorises it.
Step 4 sends the ephemeral key, `Stripe-Version: 2026-06-24.dahlia` and
`Stripe-Account: {acct}` on every line page. The key is short-lived and is
never written to a record, a state message or a log line.

Follow a URL inside the run that fetched it. Stripe expires a hosted invoice URL
30 days after the due date, and claude.ai re-issues a fresh one on every list
call — so storing a URL and following it later works in every test and then
fails in production, oldest invoices first.

**What the connector must be allowed to reach.** Steps 3 to 5 leave the cluster
for `invoicedata.stripe.com` and `api.stripe.com`; where a `NetworkPolicy`
governs egress it has to admit both. A blocked host surfaces as a chain step
that did not complete, not as missing money.

## What a line means

Categories come from the Stripe parent, never from the description — a
subscription-item parent is the recurring seat charge, an invoice-item parent is
prepaid extra usage.

A seat price is `hosted_invoice_unit_amount` on a non-proration subscription
line. It is not `amount / quantity`: a mid-period seat change emits proration
lines whose amounts cover part of a period, and dividing those yields a number
that is not a price. It is also not the wrapper's `num_seats`, which may be
absent and, where present, reports one line's quantity while an invoice can
price several tiers.

An invoice may price several tiers at once, each on its own line — so a seat
price binds to a tier and reaches a person through `class_ai_overage.seat_tier`,
never by dividing an invoice total.

`period` dates a line to the window it charges for, which is not always the
window the invoice was raised in. A line is filed by its period, never by the
invoice date.

Prepaid extra-usage purchases arrive as invoices of their own, carrying a single
`overusage` line at `quantity: 1`. They are the invoiced-layer counterpart of
the extra usage a seat later consumes, and they price no seat.

## Degradation

Every invoice emits its own row carrying its money and how far its chain got;
lines are added beside it only when the chain completed. So a chain failure keeps
the money on the ledger without a fabricated price, and the invoice keeps one row
across attempts — a later run that enriches it replaces that row instead of
adding its money a second time.

A draft is not invoiced and is skipped entirely. Its total can still change and
it carries no payment intent, and both are part of the key an invoice's row is
identified by — so emitting one would leave the draft's copy standing beside the
finalised invoice, counting that money twice.

`chain_status` distinguishes four outcomes: `ok`, `failed` (a hop answered
badly), `unparsable_url` (a hosted URL was offered but no longer matches), and
`no_hosted_url` (none was offered for an invoice the vendor has finalised). Only
URLs that were offered count towards drift: if more than half of them fail to
parse the run fails instead, because that is a format change, and a run of
unpriced rows would read as the vendor having stopped charging for seats.

## What a later sync repeats

The listing is read in full every sync, and every invoice emits its own row from
it — so the money the wrapper reports is always current, and source freshness,
which watches how recently anything arrived, keeps its anchor.

The chain is what a later sync skips. An invoice it has already followed to the
end is recognised by the identity decoded from its hosted URL, and the two Stripe
hops are not repeated: its lines are in bronze, and a settled invoice's lines do
not move. The connector remembers only what the listing cannot supply — the
invoice id its bronze rows are keyed under, and the period those lines gave.

A chain that failed is not remembered, so it is tried again on the next sync.
Losing the memory entirely — a connection recreated, say — costs one expensive
sync and nothing else: every invoice is simply chained again.
