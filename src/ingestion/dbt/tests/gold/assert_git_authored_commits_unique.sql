-- Build-integrity check (untagged → error severity under `dbt build`).
-- One row per (tenant_id, data_source, commit_hash): the collapse's `LIMIT 1 BY`
-- makes that true by construction, so a second row means a commit counts twice
-- in every commit and line measure that attaches through this set.
SELECT
    tenant_id,
    data_source,
    commit_hash,
    count() AS copy_count
FROM {{ ref('git_authored_commits') }}
GROUP BY tenant_id, data_source, commit_hash
HAVING count() > 1
