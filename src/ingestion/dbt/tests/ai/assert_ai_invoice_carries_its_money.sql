{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'A vendor invoice whose chain completed carries the money it charged',
        'domain': 'ai',
        'category': 'completeness',
        'tier': 'error',
        'remediation': 'A row here is an invoice the connector enriched successfully and that still has no money on it: the wrapper reported no total_excluding_tax for it, so invoice_net_cents is NULL and that invoice contributes nothing to invoiced spend. The gross total the wrapper did report is shown beside it — if that is present too, the field is absent rather than the invoice being empty, and the wrapper changed shape. Compare the invoice on the vendor portal against what invoice_metrics_json carries; a genuinely zero-value invoice reports 0 and does not appear here.'
    }
) }}
{#- The coverage check beside this one reports invoices whose chain did NOT
    complete. This reports the opposite shape, which that one cannot see: a chain
    that completed over an invoice the wrapper never priced. Nothing on such a row
    says it is incomplete — chain_status reads `ok`, the lines are there, and only
    the sum is quietly short.

    Bounded to the same three billing months and for the same reason: the wrapper
    either reports the field for an invoice or it never will, so an unbounded
    check would report the same historical rows forever and stop being a signal. -#}

SELECT
    insight_tenant_id,
    source,
    invoice_id,
    period_month,
    currency,
    JSONExtractString(invoice_metrics_json, 'invoice_total') AS wrapper_total
FROM {{ ref('class_ai_invoice') }} FINAL
WHERE ifNull(line_id, '') = ''
  AND chain_status = 'ok'
  AND invoice_net_cents IS NULL
  AND period_month >= toStartOfMonth(today()) - INTERVAL 2 MONTH
