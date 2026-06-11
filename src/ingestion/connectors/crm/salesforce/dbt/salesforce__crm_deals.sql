-- depends_on: {{ ref('salesforce__bronze_promoted') }}
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by='(unique_key)',
    settings={'allow_nullable_key': 1},
    tags=['salesforce', 'silver:class_crm_deals']
) }}

WITH src AS (
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
        -- SF's `Amount` is in record-currency. Single-currency orgs treat it
        -- as home; multi-currency orgs surface ConvertedAmount but we don't
        -- assume that mode here. Aliasing keeps gold per-rep aggregates
        -- comparable across connectors; tenants on multi-currency setups can
        -- swap this for ConvertedAmount at silver. `acv/tcv/arr` have no
        -- native SF equivalent (HubSpot computes them from line items).
        Amount                                          AS amount_home,
        CAST(NULL AS Nullable(Float64))                 AS acv,
        CAST(NULL AS Nullable(Float64))                 AS tcv,
        CAST(NULL AS Nullable(Float64))                 AS arr,
        CloseDate                                       AS close_date,
        OwnerId                                         AS owner_id,
        -- Rep who created the Opportunity record (universal SF audit field).
        -- Parallels HubSpot's `properties_hs_created_by_user_id`.
        CAST(CreatedById AS Nullable(String))           AS created_by_user_id,
        AccountId                                       AS account_id,
        toInt64(IsClosed = true)                        AS is_closed,
        toInt64(IsWon = true)                           AS is_won,
        LeadSource                                      AS lead_source,
        Probability                                     AS probability,
        Type                                            AS deal_type,
        -- SF has no built-in "closed lost reason" — orgs use a custom field
        -- (e.g. LossReason__c) that varies per tenant. Expose NULL; tenants
        -- with the custom field can override at gold.
        CAST(NULL AS Nullable(String))                  AS lost_reason,
        -- SF has no native "pipeline" concept (record-types fill this role
        -- but aren't a stable cross-org primitive). Expose NULL; pipeline
        -- scoping at gold uses StageName / ForecastCategory.
        CAST(NULL AS Nullable(String))                  AS pipeline_id,
        toJSONString(map(
            'Type',      coalesce(toString(Type), ''),
            'IsDeleted', if(coalesce(IsDeleted, false), 'true', 'false')
        ))                                              AS metadata,
        custom_fields,
        CreatedDate                                     AS created_at,
        LastModifiedDate                                AS updated_at,
        data_source,
        coalesce(toUnixTimestamp64Milli(SystemModstamp), 0) AS _version
    FROM {{ source('bronze_salesforce', 'Opportunity') }}
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
