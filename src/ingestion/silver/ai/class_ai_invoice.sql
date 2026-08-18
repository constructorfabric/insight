{{ config(
    materialized='incremental',
    incremental_strategy='delete+insert',
    unique_key='unique_key',
    schema='silver',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['silver']
) }}

-- Unified vendor invoices across AI vendors — the invoiced layer, as opposed to
-- consumption priced at vendor rates (class_ai_dev_usage) or spend against a
-- seat's ceiling (class_ai_overage). Grain: one row per (tenant, source, invoice)
-- carrying that invoice's own money, plus one row per (tenant, source, invoice,
-- line). Monetary contract in minor units + ISO currency. invoice_net_cents is
-- set on the invoice's row alone; seat_unit_cents is the per-seat price the
-- vendor stated on a line, NULL wherever the line prices no seat.
--
-- depends_on: {{ ref('claude_team__ai_invoice') }}

SELECT * FROM (
    {{ union_by_tag('silver:class_ai_invoice') }}
)
{% if is_incremental() %}
WHERE _version > (SELECT max(_version) FROM {{ this }})
{% endif %}
