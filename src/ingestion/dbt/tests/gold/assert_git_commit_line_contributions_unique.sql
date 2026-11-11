SELECT
    tenant_id,
    source_key,
    measure_key,
    entity_id,
    metric_date,
    record_id,
    count() AS row_count
FROM {{ ref('git_commit_line_contributions') }}
GROUP BY tenant_id, source_key, measure_key, entity_id, metric_date, record_id
HAVING count() > 1
