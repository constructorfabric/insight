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
-- so an archive event always outranks the prior live update. The archived
-- sibling is only synced when Airbyte's HubSpot connector is configured to
-- backfill deleted records — guard the UNION with adapter.get_relation so
-- absent archived tables don't break the build. Derive the bronze schema
-- from the dbt source so a tenant-prefixed `bronze_hubspot_<tenant>` rename
-- doesn't silently drop the archived UNION arm.
{%- set bronze_schema = source('bronze_hubspot', 'deals').schema -%}
{%- set bronze_tables = ['deals'] -%}
{%- if adapter.get_relation(database=none, schema=bronze_schema, identifier='deals_archived') -%}
  {%- do bronze_tables.append('deals_archived') -%}
{%- endif %}

WITH src AS (
    {% for tbl in bronze_tables %}
    SELECT
        tenant_id,
        source_id,
        unique_key,
        id                                              AS deal_id,
        properties_dealname                             AS name,
        properties_hs_manual_forecast_category          AS forecast_category,
        properties_dealstage                            AS stage,
        -- `amount` is the raw deal-currency line-item value (Constructor's
        -- school subscriptions are often quoted monthly here). `amount_home`
        -- is HubSpot's home-currency conversion — use this for cross-rep
        -- aggregates. `hs_acv` / `hs_tcv` / `hs_arr` are HubSpot-computed
        -- contract-level rollups from line items; populated on ~10% of
        -- Constructor's deals, so expose them as nullable for selective use.
        toFloat64OrNull(properties_amount)                    AS amount,
        toFloat64OrNull(properties_amount_in_home_currency)   AS amount_home,
        toFloat64OrNull(properties_hs_acv)                    AS acv,
        toFloat64OrNull(properties_hs_tcv)                    AS tcv,
        toFloat64OrNull(properties_hs_arr)                    AS arr,
        -- `properties_closedate` is an ISO datetime string (e.g.
        -- "2025-10-23T08:49:39Z"). `toDateOrNull` only handles
        -- YYYY-MM-DD; we parse via `parseDateTime64BestEffortOrNull`
        -- first then truncate to Date.
        toDate(parseDateTime64BestEffortOrNull(properties_closedate)) AS close_date,
        properties_hubspot_owner_id                     AS owner_id,
        -- Rep who logged / created the deal — distinct from owner_id, which
        -- can be the contact owner. Resolves to silver.class_crm_users
        -- via the `hs_user_id` column (HubSpot Owners API's `userId`).
        properties_hs_created_by_user_id                AS created_by_user_id,
        nullIf(arrayElement(
            JSONExtract(coalesce(associations_companies, '[]'), 'Array(String)'), 1
        ), '')                                          AS account_id,
        toInt64(coalesce(properties_hs_is_closed, 'false') = 'true')     AS is_closed,
        toInt64(coalesce(properties_hs_is_closed_won, 'false') = 'true') AS is_won,
        properties_hs_analytics_source                  AS lead_source,
        toFloat64OrNull(properties_hs_deal_stage_probability) AS probability,
        properties_dealtype                             AS deal_type,
        properties_closed_lost_reason                   AS lost_reason,
        properties_pipeline                             AS pipeline_id,
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
