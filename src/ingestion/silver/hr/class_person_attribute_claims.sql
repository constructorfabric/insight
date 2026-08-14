-- depends_on: {{ ref('bamboohr__person_attribute_claims') }}
-- depends_on: {{ ref('ms_entra__person_attribute_claims') }}
-- depends_on: {{ ref('active_directory__person_attribute_claims') }}

{{ config(
    materialized='incremental',
    incremental_strategy='delete+insert',
    unique_key='unique_key',
    schema='silver',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['silver']
) }}

-- Watermark is per source instance, not global: producers stamp _version from
-- their own snapshot clock and sync on independent schedules, so a single
-- max(_version) across the union would permanently skip a slower connector's
-- claims once a faster one advances it.
SELECT candidate.*
FROM (
    {{ union_by_tag('silver:class_person_attribute_claims') }}
) AS candidate
{% if is_incremental() %}
LEFT JOIN (
    SELECT
        insight_tenant_id,
        insight_source_id,
        max(_version) AS max_version
    FROM {{ this }}
    GROUP BY
        insight_tenant_id,
        insight_source_id
) AS watermarks
    ON  candidate.insight_tenant_id = watermarks.insight_tenant_id
    AND candidate.insight_source_id = watermarks.insight_source_id
WHERE candidate._version > coalesce(watermarks.max_version, 0)
{% endif %}
