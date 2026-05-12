-- depends_on: {{ ref('jira__bronze_promoted') }}
{{ config(
    materialized='incremental',
    alias='jira__task_field_metadata',
    incremental_strategy='append',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['jira', 'silver:class_task_field_metadata']
) }}

-- Jira field metadata → `staging.jira__task_field_metadata` → unioned into
-- `silver.class_task_field_metadata`. Classifies every field by cardinality / id-ness.
-- Bronze `jira_fields` stores schema as three flat columns: schema_type, schema_items, schema_custom.
--   is_multi  = (schema_type == 'array')
--   has_id    = multi+items!='string' OR single+items is present (structured field)
--
-- `_version` is `_airbyte_extracted_at` — deterministic and monotonic per Airbyte emission.
-- `observed_at` stays as an informational column but is NOT part of the ORDER BY, so
-- re-observing the same field across runs collapses to one row after ReplacingMergeTree
-- merge (keeping the newest).

SELECT
    f.unique_key                                  AS unique_key,
    COALESCE(f.source_id, '')                     AS insight_source_id,
    CAST('jira' AS String)                        AS data_source,
    CAST(NULL AS Nullable(String))                AS project_key,
    COALESCE(f.field_id, '')                      AS field_id,
    COALESCE(f.name, '')                          AS field_name,
    toUInt8(COALESCE(f.schema_type, '') = 'array')  AS is_multi,
    COALESCE(f.schema_type, '')                     AS field_type,
    toUInt8(
        CASE
            WHEN COALESCE(f.schema_type, '') = 'array' AND COALESCE(f.schema_items, '') = 'string' THEN 0
            WHEN COALESCE(f.schema_type, '') IN ('string', 'number', 'date', 'datetime')
                 AND COALESCE(f.schema_items, '') = '' THEN 0
            ELSE 1
        END
    )                                               AS has_id,
    toDateTime64(f._airbyte_extracted_at, 3)      AS observed_at,
    toUnixTimestamp64Milli(f._airbyte_extracted_at) AS _version
FROM {{ source('bronze_jira', 'jira_fields') }} f
-- `jira_fields` bronze = MergeTree (full_refresh + overwrite), FINAL not supported.
