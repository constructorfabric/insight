-- One row per changed path per commit: a path that appears twice in one commit
-- would double every line measure built on this dataset.
SELECT
    tenant_id,
    source,
    commit_hash,
    file_path,
    change_type,
    count() AS row_count
FROM {{ ref('git_file_changes') }}
GROUP BY tenant_id, source, commit_hash, file_path, change_type
HAVING count() > 1
