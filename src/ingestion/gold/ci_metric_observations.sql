{{ metric_observations_table() }}

-- Tenant-grain: entity_id is the tenant id (no identity resolution to do),
-- so unlike the person-grain observations there is no '' filter — every
-- evidence row serves.
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
    toNullable(sum(contribution)) AS value,
    CAST(NULL AS Nullable(String)) AS subject_key,
    dimensions
FROM {{ ref('ci_metric_evidence') }}
WHERE measure_key != 'run_duration_min'
GROUP BY tenant_id, source_key, entity_type, entity_id, account_source_type, account_source_id, account_id, metric_date, measure_key, dimensions

UNION ALL

-- Duration stays at event grain: median/percentile metrics aggregate over
-- individual runs at query time, and a pre-summed day would poison them.
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
    contribution AS value,
    subject_key,
    dimensions
FROM {{ ref('ci_metric_evidence') }}
WHERE measure_key = 'run_duration_min'
