{{ metric_observations_table() }}

SELECT
    tenant_id,
    source_key,
    entity_type,
    entity_id,
    '' AS account_source_type,
    '' AS account_source_id,
    '' AS account_id,
    metric_date,
    CAST(NULL AS Nullable(DateTime64(3))) AS observed_at,
    measure_key,
    -- Every measure is additive across a person's several seats: two seats
    -- spend two amounts, cost two fees, and their ceilings bound two seats. A
    -- window spanning months adds each month's closing figure, which is the
    -- money incurred in the window.
    toNullable(sum(contribution)) AS value,
    subject_key,
    dimensions
FROM {{ ref('ai_cost_metric_evidence') }}
GROUP BY tenant_id, source_key, entity_type, entity_id, metric_date, measure_key, subject_key, dimensions
