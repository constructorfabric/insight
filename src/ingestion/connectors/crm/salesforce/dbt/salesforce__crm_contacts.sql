{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by='(unique_key)',
    settings={'allow_nullable_key': 1},
    tags=['salesforce', 'silver:class_crm_contacts']
) }}

SELECT * FROM (
    SELECT
        tenant_id,
        source_id,
        unique_key,
        Id                                              AS contact_id,
        Email                                           AS email,
        FirstName                                       AS first_name,
        LastName                                        AS last_name,
        OwnerId                                         AS owner_id,
        AccountId                                       AS account_id,
        CAST(NULL AS Nullable(String))                  AS lifecycle_stage,
        toJSONString(map(
            'Title',      coalesce(toString(Title), ''),
            'Phone',      coalesce(toString(Phone), ''),
            'LeadSource', coalesce(toString(LeadSource), ''),
            'IsDeleted',  toString(coalesce(IsDeleted, false))
        ))                                              AS metadata,
        custom_fields,
        parseDateTimeBestEffort(CreatedDate)            AS created_at,
        parseDateTimeBestEffort(LastModifiedDate)       AS updated_at,
        data_source,
        toUnixTimestamp64Milli(
            parseDateTime64BestEffort(SystemModstamp)
        )                                               AS _version
    FROM {{ source('bronze_salesforce', 'Contact') }}
)
{% if is_incremental() %}
WHERE _version > coalesce((SELECT max(_version) FROM {{ this }}), 0)
{% endif %}
