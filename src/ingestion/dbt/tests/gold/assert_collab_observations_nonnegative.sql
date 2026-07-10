-- Build-integrity check (untagged → error severity under `dbt build`).
-- Every collaboration measure is a count, hour total, clamped message
-- difference, or scheduled-hours figure — all non-negative by construction.
-- A negative value is a regression in the gold model, not a data condition.
SELECT
    measure_key,
    count() AS row_count
FROM {{ ref('collab_metric_observations') }}
WHERE value < 0
GROUP BY measure_key
