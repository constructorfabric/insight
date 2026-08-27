{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'Recent vendor invoices carry their line items',
        'domain': 'ai',
        'category': 'coverage',
        'tier': 'error',
        'remediation': 'A row here is an invoice whose Stripe hosted chain did not complete, so its money is on the ledger with no per-seat price behind it and ai.seat_cost is missing that tier for that month. chain_status says why: no_hosted_url means the vendor offered no hosted URL for an invoice it has finalised, which is a vendor-side change rather than a fault of ours, since drafts are never emitted; unparsable_url means a URL was offered but did not match the expected form, which for more than an odd invoice means the format changed and the connector needs updating; failed means a hop answered badly — check egress to invoicedata.stripe.com and api.stripe.com, and that the pinned Stripe-Version is still accepted. Re-running the connector re-follows a freshly issued URL, and the invoice keeps one row across attempts, so a transient failure clears itself on the next sync.'
    }
) }}
{#- Bounded to the last three billing months on purpose. An invoice that never
    enriched does not self-heal, so an unbounded check would report the same
    historical rows forever and stop being a signal. Recent gaps are the
    actionable ones: the wrapper re-issues a fresh URL on every run, so a
    recent failure is one the next sync can still fix.

    No status is excluded: the connector does not emit drafts, so everything that
    reaches this class is an invoice the vendor has finalised, and any of those
    without lines is worth reporting. -#}

SELECT
    insight_tenant_id,
    source,
    invoice_id,
    period_month,
    chain_status,
    invoice_net_cents
FROM {{ ref('class_ai_invoice') }} FINAL
WHERE chain_status != 'ok'
  AND period_month >= toStartOfMonth(today()) - INTERVAL 2 MONTH
