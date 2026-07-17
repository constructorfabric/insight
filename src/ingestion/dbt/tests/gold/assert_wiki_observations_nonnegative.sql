-- Build-integrity check (untagged → error severity under `dbt build`).
-- Every wiki measure is a count (pages, edit sessions, distinct pages,
-- comments) — non-negative by construction. A negative value is a
-- regression in the gold model, not a data condition.
SELECT
    measure_key,
    count() AS row_count
FROM {{ ref('wiki_metric_observations') }}
WHERE value < 0
GROUP BY measure_key
