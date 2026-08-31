-- A request id is unique only within the repository it was opened against.
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
