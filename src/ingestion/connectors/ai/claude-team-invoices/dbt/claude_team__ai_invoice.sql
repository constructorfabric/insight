-- depends_on: {{ ref('claude_team_invoices__bronze_promoted') }}
-- Bronze → Silver: Claude Team vendor invoices → class_ai_invoice
--
-- Source: bronze_claude_team_invoices.claude_team_invoice_lines — one row per
-- invoice plus one per invoice line, produced by the CDK connector that walks the
-- claude.ai invoice wrapper and follows the Stripe hosted chain behind it.
--
-- This is the FIRST contributor to class_ai_invoice and therefore DEFINES its
-- positional contract (consumed by union_by_tag('silver:class_ai_invoice')).
-- Vendor-specific extras go into invoice_metrics_json, never new columns.
--
-- GRAIN: one row per (tenant, source, invoice) carrying that invoice's own
-- money, plus one row per (tenant, source, invoice, line). Invoice money is on
-- the invoice's row alone, so summing it needs no dedup and an invoice whose
-- chain later completes replaces its own row instead of adding a second one.
-- Aggregating to category is gold's job — a class that pre-aggregated would make
-- the per-tier seat price unrecoverable.
--
-- UNITS: Stripe amounts are ALREADY minor units (cents), so they map straight
-- through with NO ×100 — same as claude_team__ai_overage, unlike
-- claude_team__ai_dev_usage.
--
-- PERIOD: dated by the window the line CHARGES for, not by the day the invoice
-- was issued — a monthly invoice is raised at the period boundary and would
-- otherwise land in the neighbouring month. Rows carrying no line fall back to
-- the invoice date.
-- STRATEGY: delete+insert, not append. A deliberate departure from the staging
-- convention (check-dbt-conventions prescribes delete+insert for silver and
-- append for staging), because this model carries an invariant the convention
-- does not anticipate: an invoice's row is REWRITTEN as its chain gets further,
-- so a later sync must replace the row an earlier one wrote rather than stand
-- beside it. Appending leaves both versions until a background merge collapses
-- them, which makes the replacement unobservable — the `unique` test reads
-- without FINAL. The models this convention was written for restate a value
-- under a key that never moves, which union_by_tag already resolves by
-- _version; that is not this case. See #2668.
{{ config(
    materialized='incremental',
    incremental_strategy='delete+insert',
    unique_key='unique_key',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    on_schema_change='append_new_columns',
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['claude-team-invoices', 'silver:class_ai_invoice']
) }}

WITH latest_per_key AS (

    -- Bronze is full-refresh+append: every sync re-emits every invoice and line
    -- under the same unique_key, and an invoice's row changes when its chain
    -- finally completes. Collapse to the freshest read per key.
    SELECT *
    FROM {{ source('bronze_claude_team_invoices', 'claude_team_invoice_lines') }}
    WHERE unique_key IS NOT NULL
      AND unique_key != ''
    ORDER BY _airbyte_extracted_at DESC
    LIMIT 1 BY unique_key

)

SELECT
    tenant_id                                           AS insight_tenant_id,
    source_id,
    unique_key,
    invoice_id,
    line_id,
    'claude'                                            AS tool,
    invoice_status,
    -- Chain outcome, carried so an unenriched invoice reads as unenriched
    -- rather than as an invoice that happened to have no lines.
    chain_status,
    category,
    tier_label,
    toUInt8(coalesce(is_proration, false))              AS is_proration,
    coalesce(currency, invoice_currency, 'usd')         AS currency,
    toStartOfMonth(toDateTime(toInt64(coalesce(period_start_ts, invoice_created_ts, 0)))) AS period_month,
    toInt64OrNull(toString(round(amount)))              AS amount_cents,
    -- The per-seat price the vendor states on the line. NULL on prorations and
    -- on extra usage — honest-NULL: neither prices a seat.
    toInt64OrNull(toString(round(seat_unit_amount)))    AS seat_unit_cents,
    toInt64OrNull(toString(round(quantity)))            AS seat_quantity,
    toInt64OrNull(toString(round(invoice_total_excluding_tax))) AS invoice_net_cents,
    -- Vendor extras kept out of the positional contract.
    toJSONString(map(
        'product_name',  ifNull(toString(product_name), ''),
        'description',   ifNull(toString(description), ''),
        'invoice_ref',   ifNull(toString(invoice_ref), ''),
        'num_seats',     ifNull(toString(invoice_num_seats), ''),
        'invoice_total', ifNull(toString(invoice_total), ''),
        'period_end_ts', ifNull(toString(period_end_ts), '')
    ))                                                  AS invoice_metrics_json,
    'claude_team'                                       AS source,
    data_source,
    CAST(_airbyte_extracted_at AS Nullable(DateTime64(3))) AS collected_at,
    toUnixTimestamp64Milli(_airbyte_extracted_at)          AS _version
FROM latest_per_key
{% if is_incremental() %}
  -- A row only ever changes because a newer read produced it, so rows read since
  -- the last build are the only ones that can carry anything new.
  --
  -- Scoped to ONE source instance, for the reason silver_incremental_watermark
  -- gives at the class above: two instances of this connector write here, each
  -- stamping its own extraction clock, and a boundary taken over the whole table
  -- lets whichever ran first put the other's rows below it FOREVER. `coalesce`
  -- to the epoch is what admits an instance the table has never seen — comparing
  -- against NULL would drop every row of every new one.
  LEFT JOIN (
      SELECT
          insight_tenant_id,
          source_id AS watermark_source_id,
          max(collected_at) AS max_collected
      FROM {{ this }}
      GROUP BY insight_tenant_id, source_id
  ) AS watermarks
      ON latest_per_key.tenant_id = watermarks.insight_tenant_id
     AND latest_per_key.source_id = watermarks.watermark_source_id
  WHERE latest_per_key._airbyte_extracted_at
      > coalesce(watermarks.max_collected, toDateTime64('1970-01-01 00:00:00', 3))
{% endif %}
