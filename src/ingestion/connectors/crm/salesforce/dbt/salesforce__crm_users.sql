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
        Email                                           AS email,
        FirstName                                       AS first_name,
        LastName                                        AS last_name,
        Title                                           AS title,
        Department                                      AS department,
        toInt64(IsActive = true)                        AS is_active,
        toJSONString(map(
            'Username',   coalesce(toString(Username), ''),
            'ProfileId',  coalesce(toString(ProfileId), ''),
            'UserRoleId', coalesce(toString(UserRoleId), ''),
            'IsDeleted',  toString(coalesce(IsDeleted, false))
        ))                                              AS metadata,
        custom_fields,
        collected_at,
        data_source,
        toUnixTimestamp64Milli(
            parseDateTime64BestEffort(SystemModstamp)
        )                                               AS _version
    FROM {{ source('bronze_salesforce', 'User') }}
)
{% if is_incremental() %}
WHERE _version > coalesce((SELECT max(_version) FROM {{ this }}), 0)
{% endif %}
