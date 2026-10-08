{{ config(materialized='table', engine=insight_engine('ReplacingMergeTree', '_version'), order_by=['unique_key'], settings={'allow_nullable_key': 1}, schema='staging', tags=['youtrack', 'staging', 'silver:class_task_field_metadata']) }}

WITH catalogue AS (
    SELECT insight_source_id, field_id, field_name, field_type,
        toUInt8(endsWith(field_type, '[*]')) AS is_multi,
        toUInt8(match(field_type, '^(enum|state|version|build|ownedField|user|group)')) AS has_id,
        observed_at
    FROM {{ ref('youtrack__field_observations') }} FINAL
    QUALIFY row_number() OVER (PARTITION BY insight_source_id, field_id ORDER BY observed_at DESC) = 1
), properties AS (
    -- Named as the snapshot and the history name them.
    SELECT insight_source_id, prop.1 AS field_id, prop.2 AS field_name, prop.3 AS field_type,
        toUInt8(prop.4) AS is_multi, toUInt8(prop.5) AS has_id, max(observed_at) AS observed_at
    FROM {{ ref('youtrack__issues') }}
    ARRAY JOIN [('summary', 'Summary', 'string', 0, 0), ('description', 'Description', 'text', 0, 0),
                ('project', 'Project', 'project', 0, 1), ('tags', 'Tags', 'tag', 1, 1),
                ('sprints', 'Sprints', 'sprint', 1, 1)] AS prop
    GROUP BY insight_source_id, prop
)
SELECT CAST(concat(insight_source_id, '-youtrack-field-', field_id) AS Nullable(String)) AS unique_key,
    insight_source_id, 'youtrack' AS data_source, CAST(NULL AS Nullable(String)) AS project_key,
    field_id, field_name, is_multi, field_type, has_id, observed_at,
    toUnixTimestamp64Milli(now64(3)) AS _version
FROM (SELECT * FROM catalogue UNION ALL SELECT * FROM properties)
