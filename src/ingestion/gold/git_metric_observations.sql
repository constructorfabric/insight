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
    toNullable({{ collapsed_value('contribution', max_keys=['commit_day']) }}) AS value,
    CAST(NULL AS Nullable(String)) AS subject_key,
    dimensions
FROM {{ ref('git_metric_evidence') }}
WHERE measure_key NOT IN ('commit_change_size', 'pr_cycle_hours', 'pr_change_size')
  AND entity_id != ''
-- One person's several source accounts collapse into one canonical row.
GROUP BY tenant_id, source_key, entity_type, entity_id, metric_date, measure_key, dimensions

UNION ALL

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
    contribution AS value,
    subject_key,
    dimensions
FROM {{ ref('git_metric_evidence') }}
WHERE measure_key IN ('commit_change_size', 'pr_cycle_hours', 'pr_change_size')
  AND entity_id != ''
