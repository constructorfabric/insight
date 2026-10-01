{{ config(materialized='table', engine='ReplacingMergeTree(_version)', order_by=['unique_key'], settings={'allow_nullable_key': 1}, schema='staging', tags=['youtrack', 'staging', 'silver:class_task_statuses']) }}

-- The status dimension: every value of a field bound to the `status` role.
-- YouTrack states whether a state value resolves an issue (`isResolved`), so
-- `done` is the vendor's fact, as Jira's status category is; whether an open
-- value means `new` or `in_progress` it does not state, so that is the
-- operator's config.task_value_map decision. A decision wins over the flag,
-- and an open value nobody has decided is `undefined` — never guessed.
WITH status_fields AS (
    SELECT insight_source_id, field_id FROM (
        SELECT * FROM {{ source('config', 'task_field_roles') }} FINAL
        WHERE data_source = 'youtrack' AND valid_from <= now64(3)
        ORDER BY valid_from DESC, recorded_at DESC
        LIMIT 1 BY insight_source_id, field_id
    ) WHERE is_deleted = 0 AND role = 'status'
), observed AS (
    SELECT v.insight_source_id AS insight_source_id, v.value_id AS value_id, v.value_name AS value_name,
        JSONExtractBool(v.value, 'isResolved') AS is_resolved
    FROM {{ ref('youtrack__bundle_values') }} AS v
    INNER JOIN status_fields AS f ON f.insight_source_id = v.insight_source_id AND f.field_id = v.field_id
    WHERE v.value_id != ''
    LIMIT 1 BY insight_source_id, value_id
), decided AS (
    SELECT insight_source_id, value_id, value_display, canonical_value FROM (
        SELECT * FROM {{ source('config', 'task_value_map') }} FINAL
        WHERE data_source = 'youtrack' AND valid_from <= now64(3)
        ORDER BY valid_from DESC, recorded_at DESC
        LIMIT 1 BY insight_source_id, field_id, value_id
    )
    WHERE is_deleted = 0 AND canonical_value IN ('new', 'in_progress', 'done', 'undefined')
    LIMIT 1 BY insight_source_id, value_id
), statuses AS (
    -- A decided value no bundle lists any more (a removed state) keeps its row.
    SELECT if(o.value_id != '', o.insight_source_id, d.insight_source_id) AS insight_source_id,
        if(o.value_id != '', o.value_id, d.value_id) AS value_id,
        coalesce(nullIf(o.value_name, ''), nullIf(d.value_display, '')) AS value_name,
        coalesce(nullIf(d.canonical_value, ''), if(o.is_resolved, 'done', 'undefined')) AS status_category
    FROM observed AS o
    FULL OUTER JOIN decided AS d ON d.insight_source_id = o.insight_source_id AND d.value_id = o.value_id
)
SELECT CAST(concat(insight_source_id, '-youtrack-status-', value_id) AS Nullable(String)) AS unique_key,
    CAST(insight_source_id AS Nullable(String)) AS insight_source_id, 'youtrack' AS data_source,
    CAST(value_id AS Nullable(String)) AS status_id, CAST(value_name AS Nullable(String)) AS status_name,
    CAST(NULL AS Nullable(Int32)) AS category_id, CAST(NULL AS Nullable(String)) AS category_key,
    CAST(status_category AS String) AS status_category, now64(3) AS collected_at,
    toUnixTimestamp64Milli(now64(3)) AS _version
FROM statuses
WHERE value_id != ''
