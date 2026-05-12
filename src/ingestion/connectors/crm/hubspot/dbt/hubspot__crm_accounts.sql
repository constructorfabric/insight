{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by='(unique_key)',
    settings={'allow_nullable_key': 1},
    tags=['hubspot', 'silver:class_crm_accounts']
) }}

-- Live (`companies`) and archived (`companies_archived`) are sibling Bronze tables;
-- ReplacingMergeTree on `unique_key` dedups, with `_version = greatest(updatedAt, archivedAt)`
-- so an archive event always outranks the prior live update.
WITH src AS (
    {% for tbl in ['companies', 'companies_archived'] %}
    SELECT
        tenant_id,
        source_id,
        unique_key,
        id                                              AS account_id,
        properties_name                                 AS name,
        properties_domain                               AS domain,
        properties_industry                             AS industry,
        properties_hubspot_owner_id                     AS owner_id,
        -- HubSpot has no native parent-account hierarchy in v3.
        CAST(NULL AS Nullable(String))                  AS parent_account_id,
        toJSONString(map(
            'city',              coalesce(toString(properties_city), ''),
            'state',             coalesce(toString(properties_state), ''),
            'country',           coalesce(toString(properties_country), ''),
            'numberofemployees', coalesce(toString(properties_numberofemployees), ''),
            'annualrevenue',     coalesce(toString(properties_annualrevenue), ''),
            'archived',          toString(coalesce(archived, false))
        ))                                              AS metadata,
        createdAt                                       AS created_at,
        updatedAt                                       AS updated_at,
        data_source,
        greatest(
            coalesce(toUnixTimestamp64Milli(updatedAt), 0),
            coalesce(toUnixTimestamp64Milli(archivedAt), 0)
        )                                               AS _version
    FROM {{ source('bronze_hubspot', tbl) }}
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
