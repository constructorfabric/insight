-- Build-integrity check (untagged → error severity under `dbt build`).
-- One row per commit within the repository that collected it. A commit reached
-- through two branches is one commit; counting it twice overstates the
-- denominator the run-to-commit coverage reading divides by.
SELECT
    tenant_id,
    source_id,
    project_key,
    repo_slug,
    commit_hash,
    count() AS row_count
FROM {{ ref('ci_commits') }}
GROUP BY tenant_id, source_id, project_key, repo_slug, commit_hash
HAVING count() > 1
