-- Build-integrity check (untagged → error severity under `dbt build`).
-- Every git measure is a count, line total, size, or clamped duration —
-- all non-negative by construction (cycle hours exclude reversed
-- timestamps at the source CTE). A negative value is a regression in the
-- gold model, not a data condition.
SELECT
    measure_key,
    count() AS row_count
FROM {{ ref('git_metric_observations') }}
WHERE value < 0
GROUP BY measure_key
