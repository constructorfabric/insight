-- One row per request per tenant and repository: a request is identified by
-- the repository it was opened against, so the same id in two repositories is
-- two requests and the same id twice in one is a duplicated read.
SELECT
    tenant_id,
    source_id,
    project_key,
    repo_slug,
    pr_id,
    count() AS row_count
FROM {{ ref('git_pull_requests') }}
GROUP BY tenant_id, source_id, project_key, repo_slug, pr_id
HAVING count() > 1
