-- Build-integrity check (untagged → error severity under `dbt build`).
-- The sum measures are day-grain: exactly one row per (tenant, entity, date,
-- measure, dimensions, subject). A duplicate means FINAL dedup regressed on a
-- class read or a measure branch fanned out, silently inflating the sums.
-- The three event measures (dev_time_hours, resolution_days, pickup_days) are
-- per-closed-issue by design — many rows per (entity, day) feed the query-time
-- median — so they are exempt from the uniqueness grain.
SELECT
    tenant_id,
    entity_id,
    metric_date,
    measure_key,
    dimensions,
    subject_key,
    count() AS row_count
FROM {{ ref('task_metric_observations') }}
WHERE measure_key NOT IN ('dev_time_hours', 'resolution_days', 'pickup_days')
GROUP BY tenant_id, entity_id, metric_date, measure_key, dimensions, subject_key
HAVING count() > 1
