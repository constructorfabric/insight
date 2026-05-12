{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by='(unique_key)',
    settings={'allow_nullable_key': 1},
    tags=['hubspot', 'silver:class_crm_deals']
) }}

-- Live (`deals`) and archived (`deals_archived`) are sibling Bronze tables;
-- ReplacingMergeTree on `unique_key` dedups, with `_version = greatest(updatedAt, archivedAt)`
-- so an archive event always outranks the prior live update.
WITH src AS (
    {% for tbl in ['deals', 'deals_archived'] %}
    SELECT
        tenant_id,
        source_id,
        unique_key,
        id                                              AS deal_id,
        properties_dealname                             AS name,
        properties_hs_manual_forecast_category          AS forecast_category,
        properties_dealstage                            AS stage,
        toFloat64OrNull(properties_amount)              AS amount,
        toDateOrNull(properties_closedate)              AS close_date,
        properties_hubspot_owner_id                     AS owner_id,
        nullIf(arrayElement(
            JSONExtract(coalesce(associations_companies, '[]'), 'Array(String)'), 1
        ), '')                                          AS account_id,
        toInt64(coalesce(properties_hs_is_closed, 'false') = 'true')     AS is_closed,
        toInt64(coalesce(properties_hs_is_closed_won, 'false') = 'true') AS is_won,
        properties_hs_analytics_source                  AS lead_source,
        toFloat64OrNull(properties_hs_deal_stage_probability) AS probability,
        toJSONString(map(
            'pipeline',       coalesce(toString(properties_pipeline), ''),
            'deal_type',      coalesce(toString(properties_dealtype), ''),
            'archived',       toString(coalesce(archived, false))
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
