-- depends_on: {{ ref('salesforce__bronze_promoted') }}
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by='(unique_key)',
    settings={'allow_nullable_key': 1},
    tags=['salesforce', 'silver:class_crm_contacts']
) }}

WITH src AS (
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
        CreatedDate                                     AS created_at,
        LastModifiedDate                                AS updated_at,
        data_source,
        coalesce(toUnixTimestamp64Milli(SystemModstamp), 0) AS _version
    FROM {{ source('bronze_salesforce', 'Contact') }}
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
