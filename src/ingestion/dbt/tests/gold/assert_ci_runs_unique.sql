-- Build-integrity check (untagged → error severity under `dbt build`).
-- One row per run: the model's `LIMIT 1 BY` keeps the latest attempt and drops
-- the rest, so a second row means a retried run is being counted once per
-- attempt and every run measure over it is inflated.
SELECT
    tenant_id,
    source_id,
    repository_value,
    run_id,
    count() AS row_count
FROM {{ ref('ci_runs') }}
GROUP BY tenant_id, source_id, repository_value, run_id
HAVING count() > 1
