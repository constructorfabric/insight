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

SELECT * FROM (
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
WHERE _version > coalesce((SELECT max(_version) FROM {{ this }}), 0)
{% endif %}
