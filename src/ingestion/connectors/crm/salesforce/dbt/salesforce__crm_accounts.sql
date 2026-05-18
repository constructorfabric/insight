{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by='(unique_key)',
    settings={'allow_nullable_key': 1},
    tags=['salesforce', 'silver:class_crm_accounts']
) }}

SELECT * FROM (
    SELECT
        tenant_id,
        source_id,
        unique_key,
        Id                                              AS account_id,
        Name                                            AS name,
        domain(Website)                                 AS domain,
        Industry                                        AS industry,
        OwnerId                                         AS owner_id,
        ParentId                                        AS parent_account_id,
        toJSONString(map(
            'Type',              coalesce(toString(Type), ''),
            'BillingCity',       coalesce(toString(BillingCity), ''),
            'BillingState',      coalesce(toString(BillingState), ''),
            'BillingCountry',    coalesce(toString(BillingCountry), ''),
            'NumberOfEmployees', coalesce(toString(NumberOfEmployees), ''),
            'AnnualRevenue',     coalesce(toString(AnnualRevenue), ''),
            'IsDeleted',         toString(coalesce(IsDeleted, false))
        ))                                              AS metadata,
        custom_fields,
        parseDateTimeBestEffort(CreatedDate)            AS created_at,
        parseDateTimeBestEffort(LastModifiedDate)       AS updated_at,
        data_source,
        toUnixTimestamp64Milli(
            parseDateTime64BestEffort(SystemModstamp)
        )                                               AS _version
    FROM {{ source('bronze_salesforce', 'Account') }}
)
{% if is_incremental() %}
WHERE _version > coalesce((SELECT max(_version) FROM {{ this }}), 0)
{% endif %}
