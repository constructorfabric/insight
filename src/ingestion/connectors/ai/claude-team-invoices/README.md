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
| 4 | `GET api.stripe.com/v1/invoices/{id}/hosted` | the invoice with its lines embedded |
| 5 | `GET api.stripe.com/v1/invoices/{id}/lines?limit=100` | the full line set, when step 4 reports more |

Steps 3 to 5 pin `Stripe-Version: 2026-06-24.dahlia` and pass
`Stripe-Account: {acct}`. The ephemeral key is short-lived, authorises the last
two hops only, and is never written to a record, a state message or a log line.

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
that is not a price. It is also not the wrapper's `num_seats`, which is absent
on most invoices and, where present, reports one line's quantity while the
invoice covers several tiers.

One tenant runs several tiers at once — a single invoice prices each of them on
its own line — so a seat price binds to a tier and reaches a person through
`class_ai_overage.seat_tier`, never by dividing an invoice total.

`period` dates a line to the window it charges for, which is not always the
window the invoice was raised in. A line is filed by its period, never by the
invoice date.

Prepaid extra-usage purchases arrive as invoices of their own, carrying a single
`overusage` line at `quantity: 1`. They are the invoiced-layer counterpart of
the extra usage a seat later consumes, and they price no seat.

## Degradation

A chain failure on one invoice emits that invoice with `chain_status = 'failed'`
and no line, so the money stays on the ledger without a fabricated price. If
more than half the hosted URLs fail to parse the run fails instead: that is a
format change, and a run of unpriced rows would read as the vendor having
stopped charging for seats.
