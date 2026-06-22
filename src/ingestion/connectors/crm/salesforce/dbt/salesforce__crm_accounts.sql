-- depends_on: {{ ref('salesforce__bronze_promoted') }}
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by='(unique_key)',
    settings={'allow_nullable_key': 1},
    tags=['salesforce', 'silver:class_crm_accounts']
) }}

WITH src AS (
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
        CreatedDate                                     AS created_at,
        LastModifiedDate                                AS updated_at,
        data_source,
        coalesce(toUnixTimestamp64Milli(SystemModstamp), 0) AS _version
    FROM {{ source('bronze_salesforce', 'Account') }}
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
