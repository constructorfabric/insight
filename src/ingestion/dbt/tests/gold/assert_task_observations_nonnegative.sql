-- Build-integrity check (untagged → error severity under `dbt build`).
-- Every task measure is a count, a duration (seconds/hours/days clamped to
-- non-negative at the interval CTE), a late-slip day count, or a ratio
-- numerator/denominator — all non-negative by construction. A negative value
-- is a regression in the gold model, not a data condition.
SELECT
    measure_key,
    count() AS row_count
FROM {{ ref('task_metric_observations') }}
WHERE value < 0
GROUP BY measure_key
