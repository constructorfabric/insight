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
