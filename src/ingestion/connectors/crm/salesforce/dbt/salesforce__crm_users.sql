-- depends_on: {{ ref('salesforce__bronze_promoted') }}
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by='(unique_key)',
    settings={'allow_nullable_key': 1},
    tags=['salesforce', 'silver:class_crm_users']
) }}

WITH src AS (
    SELECT
        tenant_id,
        source_id,
        unique_key,
        Id                                              AS user_id,
        -- HubSpot Owners API exposes two IDs (owners.id + owners.userId);
        -- SF's User.Id is canonical (no parallel identifier). Emit NULL so
        -- the column shape matches HubSpot at silver UNION ALL.
        CAST(NULL AS Nullable(String))                  AS hs_user_id,
        Email                                           AS email,
        FirstName                                       AS first_name,
        LastName                                        AS last_name,
        Title                                           AS title,
        Department                                      AS department,
        toInt64(IsActive = true)                        AS is_active,
        toJSONString(map(
            'Username',   coalesce(toString(Username), ''),
            'UserRoleId', coalesce(toString(UserRoleId), '')
        ))                                              AS metadata,
        custom_fields,
        collected_at,
        data_source,
        coalesce(toUnixTimestamp64Milli(SystemModstamp), 0) AS _version
    FROM {{ source('bronze_salesforce', 'User') }}
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
