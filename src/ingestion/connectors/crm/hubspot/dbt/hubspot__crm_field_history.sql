{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by='(unique_key)',
    settings={'allow_nullable_key': 1},
    tags=['hubspot', 'silver:class_crm_field_history']
) }}

-- One row per property value change. Changes are immutable, so the change
-- time versions the row and a replayed history collapses onto it.
WITH src AS (
    {% for tbl in ['deals_property_history', 'tickets_property_history'] %}
    SELECT
        tenant_id,
        source_id,
        concat(tenant_id, '-', source_id, '-', objectType, '-', id) AS unique_key,
        objectType                                      AS object_type,
        objectId                                        AS object_id,
        propertyName                                    AS field,
        value,
        timestamp                                       AS changed_at,
        sourceType                                      AS change_source_type,
        sourceId                                        AS change_source_id,
        sourceLabel                                     AS change_source_label,
        toString(updatedByUserId)                       AS changed_by_user_id,
        data_source,
        coalesce(toUnixTimestamp64Milli(timestamp), 0)  AS _version
    FROM {{ source('bronze_hubspot', tbl) }}
    {% if not loop.last %}UNION ALL{% endif %}
    {% endfor %}
)

{{ hubspot_staging_tail('src') }}
