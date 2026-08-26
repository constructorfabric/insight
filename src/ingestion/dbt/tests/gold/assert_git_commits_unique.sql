-- One row per commit per tenant and source system: the instance dedup is what
-- keeps a mirrored repository from multiplying a person's work.
SELECT
    tenant_id,
    source,
    commit_hash,
    count() AS row_count
FROM {{ ref('git_commits') }}
GROUP BY tenant_id, source, commit_hash
HAVING count() > 1
