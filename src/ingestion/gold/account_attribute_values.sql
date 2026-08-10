{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['insight_tenant_id', 'insight_source_type', 'insight_source_id', 'source_account_id', 'field_id', 'valid_from'],
    schema=var('gold_database'),
    alias='account_attribute_values',
    tags=['gold'],
    query_settings={
        'max_memory_usage': 3221225472,
        'max_threads': 4
    }
) }}

-- 'clear' claims participate in the window as interval closers, then drop in
-- the final filter — a cleared attribute has no current row, not an empty one.

WITH claims AS (
    SELECT
        insight_tenant_id,
        insight_source_type,
        insight_source_id,
        source_account_id,
        field_id,
        value_id,
        value_label,
        claim_action,
        observed_at,
        ingested_at
    FROM {{ ref('class_person_attribute_claims') }} FINAL
),

intervals AS (
    SELECT
        *,
        leadInFrame(toNullable(observed_at)) OVER (
            PARTITION BY
                insight_tenant_id,
                insight_source_type,
                insight_source_id,
                source_account_id,
                field_id
            ORDER BY observed_at
            ROWS BETWEEN 1 FOLLOWING AND 1 FOLLOWING
        ) AS next_observed_at
    FROM claims
)

SELECT
    insight_tenant_id,
    insight_source_type,
    insight_source_id,
    source_account_id,
    field_id,
    value_id,
    value_label,
    observed_at       AS valid_from,
    next_observed_at  AS valid_to,
    ingested_at
FROM intervals
WHERE claim_action = 'set'
