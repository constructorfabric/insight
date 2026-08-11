{{ metric_observations_table() }}

SELECT
    tenant_id,
    source_key,
    entity_type,
    -- entity_id arrives ALREADY canonical from evidence (resolved once per
    -- build); '' marks a row identity could not resolve, which stays out of
    -- every serving relation and is counted by identity_resolution_coverage.
    entity_id,
    metric_date,
    CAST(NULL AS Nullable(DateTime64(3))) AS observed_at,
    measure_key,
    -- Both measures are additive across a person's several seats: two seats
    -- spend two amounts, and their ceilings bound two seats. A window spanning
    -- months adds each month's closing figure, which is the money incurred in
    -- the window.
    toNullable({{ collapsed_value('contribution') }}) AS value,
    subject_key,
    dimensions
FROM {{ ref('ai_cost_metric_evidence') }}
WHERE entity_id != ''
GROUP BY tenant_id, source_key, entity_type, entity_id, metric_date, measure_key, subject_key, dimensions
