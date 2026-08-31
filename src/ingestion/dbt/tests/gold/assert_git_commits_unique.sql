SELECT
    tenant_id,
    source,
    commit_hash,
    count() AS row_count
FROM {{ ref('git_commits') }}
GROUP BY tenant_id, source, commit_hash
HAVING count() > 1
