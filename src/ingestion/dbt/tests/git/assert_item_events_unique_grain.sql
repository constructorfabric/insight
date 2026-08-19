-- Build-integrity guard: at most one row per (tenant, source, item, event).
-- A surplus row means two producers encoded the same vendor event under
-- different unique_key values, so RMT cannot collapse them and a consumer
-- folding a multi-valued field would apply the same add/remove twice.
SELECT
    tenant_id,
    source_id,
    item_type,
    item_number,
    event_id,
    count()               AS row_count,
    uniqExact(unique_key) AS distinct_unique_keys
FROM {{ ref('class_git_item_events') }} FINAL
GROUP BY
    tenant_id,
    source_id,
    item_type,
    item_number,
    event_id
HAVING count() > 1
