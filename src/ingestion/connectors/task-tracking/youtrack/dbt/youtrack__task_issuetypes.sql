-- depends_on: {{ ref('youtrack__bronze_promoted') }}
{{ config(
    materialized='view',
    alias='youtrack__task_issuetypes',
    schema='staging',
    tags=['youtrack', 'silver:class_task_issuetypes']
) }}

-- Per-source issue-type dimension; unioned into `silver.class_task_issuetypes`
-- via `union_by_tag`. YouTrack has no global issue-type table: the instance's
-- types are the Type custom field's enum-bundle values, selected by
-- `field_name` (canonical) rather than `field_localized_name` (per-language).
-- No untranslated name exists here, so a non-default type set reads `unknown`.
--
-- `bundle_values_json` is the raw JSON array of bundle value objects. A bundle
-- may be shared across projects, so the same value id appears in several rows;
-- `union_by_tag` dedups by `unique_key`.

WITH type_fields AS (
    SELECT
        pcf.source_id                                           AS source_id,
        pcf.bundle_values_json                                  AS bundle_values_json,
        pcf._airbyte_extracted_at                               AS _airbyte_extracted_at
    FROM {{ source('bronze_youtrack', 'youtrack_project_custom_fields') }} pcf
    WHERE has({{ task_type_name_array(var('task_issue_type_field_names')) }},
              lower(trimBoth(ifNull(toString(pcf.field_name), ''))))
      AND (lower(toString(pcf.value_type)) = 'enum'
           OR toString(pcf.field_type_id) LIKE 'enum%')
)
SELECT
    concat(toString(tf.source_id), '-', JSONExtractString(val_raw, 'id')) AS unique_key,
    tf.source_id                                                AS insight_source_id,
    CAST('youtrack' AS String)                                  AS data_source,
    JSONExtractString(val_raw, 'id')                            AS issue_type_id,
    JSONExtractString(val_raw, 'name')                          AS issue_type_name,
    CAST(NULL AS Nullable(String))                              AS untranslated_name,
    {{ task_issue_kind("JSONExtractString(val_raw, 'name')") }} AS issue_kind,
    toDateTime64(tf._airbyte_extracted_at, 3)                   AS collected_at,
    toUnixTimestamp64Milli(now64(3))                            AS _version
FROM type_fields tf
ARRAY JOIN JSONExtractArrayRaw(ifNull(tf.bundle_values_json, '[]')) AS val_raw
WHERE JSONExtractString(val_raw, 'id') != ''
