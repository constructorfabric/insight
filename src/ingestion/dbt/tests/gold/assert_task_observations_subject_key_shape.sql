-- Build-integrity check (untagged → error severity under `dbt build`).
-- The task family has no distinct-count measure, so the model is a single
-- value/event UNION branch that stamps subject_key = NULL on every row. A
-- non-NULL subject_key means a measure branch drifted into a distinct-count
-- shape — nothing downstream would count it, so it must never appear.
SELECT
    measure_key,
    countIf(subject_key IS NOT NULL) AS subject_rows
FROM {{ ref('task_metric_observations') }}
GROUP BY measure_key
HAVING subject_rows > 0
