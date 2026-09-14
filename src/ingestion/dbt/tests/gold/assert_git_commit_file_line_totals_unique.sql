-- Build-integrity check (untagged → error severity under `dbt build`).
-- git_metric_evidence LEFT JOINs this relation on (tenant, data_source,
-- commit_hash) to read a commit's collected size. A second row per key fans
-- that join out and every git size measure counts the commit twice. The grain
-- holds by construction; a widened GROUP BY is what would break it.
SELECT
    tenant_id,
    data_source,
    commit_hash,
    count() AS row_count
FROM {{ ref('git_commit_file_line_totals') }}
GROUP BY tenant_id, data_source, commit_hash
HAVING count() > 1
