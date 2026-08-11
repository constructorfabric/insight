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

```
GET {proxy}/api/stripe/{org}/invoices            -> wrapper rows, hosted_invoice_url
parse https://invoice.stripe.com/i/{acct}/{tok}  -> (acct, token)
GET invoicedata.stripe.com/hosted_invoice_page/… -> invoice_id + ephemeral key
GET api.stripe.com/v1/invoices/{id}/lines        -> line items
```

The ephemeral key is short-lived, authorises the last hop only, and is never
written to a record, a state message or a log line.

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

One tenant runs several tiers at once, so a seat price binds to a tier — see
`docs/domain/metrics/specs/ai-cost/research-notes.md` §20 for the measurements.

## Degradation

A chain failure on one invoice emits that invoice with `chain_status = 'failed'`
and no line, so the money stays on the ledger without a fabricated price. If
more than half the hosted URLs fail to parse the run fails instead: that is a
format change, and a run of unpriced rows would read as the vendor having
stopped charging for seats.
