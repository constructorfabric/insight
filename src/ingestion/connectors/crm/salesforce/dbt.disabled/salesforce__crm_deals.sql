{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by='(unique_key)',
    settings={'allow_nullable_key': 1},
    tags=['salesforce', 'silver:class_crm_deals']
) }}

SELECT * FROM (
    SELECT
        tenant_id,
        source_id,
        unique_key,
        Id                                              AS deal_id,
        Name                                            AS name,
        -- SF has no native "pipeline" concept; ForecastCategory
        -- (Pipeline/BestCase/Commit/Closed) is the closest bucketing.
        -- Real pipeline semantics are derived at Silver from StageName.
        ForecastCategory                                AS forecast_category,
        StageName                                       AS stage,
        Amount                                          AS amount,
        toDateOrNull(CloseDate)                         AS close_date,
        OwnerId                                         AS owner_id,
        AccountId                                       AS account_id,
        toInt64(IsClosed = true)                        AS is_closed,
        toInt64(IsWon = true)                           AS is_won,
        LeadSource                                      AS lead_source,
        Probability                                     AS probability,
        toJSONString(map(
            'Type',      coalesce(toString(Type), ''),
            'IsDeleted', if(coalesce(IsDeleted, false), 'true', 'false')
        ))                                              AS metadata,
        custom_fields,
        parseDateTime64BestEffortOrNull(CreatedDate, 3)      AS created_at,
        parseDateTime64BestEffortOrNull(LastModifiedDate, 3) AS updated_at,
        data_source,
        toUnixTimestamp64Milli(
            parseDateTime64BestEffort(SystemModstamp)
        )                                               AS _version
    FROM {{ source('bronze_salesforce', 'Opportunity') }}
)
{% if is_incremental() %}
WHERE _version > coalesce((SELECT max(_version) FROM {{ this }}), 0)
{% endif %}
