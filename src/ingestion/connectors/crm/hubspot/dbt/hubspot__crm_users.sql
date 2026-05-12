{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by='(unique_key)',
    settings={'allow_nullable_key': 1},
    tags=['hubspot', 'silver:class_crm_users']
) }}

-- Live (`owners`) and archived (`owners_archived`) are sibling Bronze tables;
-- ReplacingMergeTree on `unique_key` dedups, with `_version = greatest(updatedAt, archivedAt)`
-- so an archive event always outranks the prior live update.
WITH src AS (
    {% for tbl in ['owners', 'owners_archived'] %}
    SELECT
        tenant_id,
        source_id,
        unique_key,
        id                                              AS user_id,
        email                                           AS email,
        firstName                                       AS first_name,
        lastName                                        AS last_name,
        -- HubSpot Owners API exposes no title/department; Silver requires
        -- the columns to exist so emit explicit NULLs.
        CAST(NULL AS Nullable(String))                  AS title,
        CAST(NULL AS Nullable(String))                  AS department,
        toInt64(NOT coalesce(archived, false))          AS is_active,
        toJSONString(map(
            'userId',   coalesce(toString(userId), ''),
            'archived', toString(coalesce(archived, false))
        ))                                              AS metadata,
        collected_at,
        data_source,
        greatest(
            coalesce(toUnixTimestamp64Milli(updatedAt), 0),
            coalesce(toUnixTimestamp64Milli(archivedAt), 0)
        )                                               AS _version
    FROM {{ source('bronze_hubspot', tbl) }}
    -- Silver class_crm_users requires email NOT NULL for identity resolution.
    -- HubSpot Owners for deactivated internal users can lack an email.
    WHERE email IS NOT NULL AND email != ''
    {% if not loop.last %}UNION ALL{% endif %}
    {% endfor %}
)
{% if is_incremental() %}
SELECT src.*
FROM src
LEFT JOIN (
    SELECT tenant_id, source_id, max(_version) AS hwm
    FROM {{ this }}
    GROUP BY tenant_id, source_id
) w
  ON w.tenant_id = src.tenant_id AND w.source_id = src.source_id
WHERE src._version > coalesce(w.hwm, 0)
{% else %}
SELECT * FROM src
{% endif %}
