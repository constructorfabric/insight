{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by='(unique_key)',
    settings={'allow_nullable_key': 1},
    tags=['hubspot', 'silver:class_crm_deals']
) }}

WITH src AS (
    {% for tbl in hubspot_bronze_tables('deals') %}
    SELECT
        tenant_id,
        source_id,
        unique_key,
        id                                              AS deal_id,
        properties_dealname                             AS name,
        properties_hs_manual_forecast_category          AS forecast_category,
        properties_dealstage                            AS stage,
        -- `amount` is the raw deal-currency line-item value and may be
        -- quoted per billing period rather than annualized. `amount_home`
        -- is HubSpot's home-currency conversion — use this for cross-rep
        -- aggregates. `hs_acv` / `hs_tcv` / `hs_arr` are HubSpot-computed
        -- contract-level rollups from line items; typically populated on
        -- only a minority of deals, so expose them as nullable for
        -- selective use.
        toFloat64OrNull(properties_amount)                    AS amount,
        toFloat64OrNull(properties_amount_in_home_currency)   AS amount_home,
        toFloat64OrNull(properties_hs_acv)                    AS acv,
        toFloat64OrNull(properties_hs_tcv)                    AS tcv,
        toFloat64OrNull(properties_hs_arr)                    AS arr,
        -- `properties_closedate` is an ISO datetime string (e.g.
        -- "2025-10-23T08:49:39Z"). `toDateOrNull` only handles
        -- YYYY-MM-DD; we parse via `parseDateTime64BestEffortOrNull`
        -- first then truncate to Date.
        toDate32(parseDateTime64BestEffortOrNull(properties_closedate)) AS close_date,
        properties_hubspot_owner_id                     AS owner_id,
        -- Rep who logged / created the deal — distinct from owner_id, which
        -- can be the contact owner. Resolves to silver.class_crm_users
        -- via the `hs_user_id` column (HubSpot Owners API's `userId`).
        properties_hs_created_by_user_id                AS created_by_user_id,
        {{ hubspot_first_association('associations_companies') }} AS account_id,
        toNullable(toInt64(coalesce(properties_hs_is_closed, 'false') = 'true'))     AS is_closed,
        toNullable(toInt64(coalesce(properties_hs_is_closed_won, 'false') = 'true')) AS is_won,
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
        {{ hubspot_record_version() }}                  AS _version,
        {{ hubspot_deletion_columns() }},
        {{ hubspot_properties_map() }}                  AS properties,
        properties_deal_currency_code                   AS currency,
        toFloat64OrNull(properties_hs_exchange_rate)    AS exchange_rate
    FROM {{ source('bronze_hubspot', tbl) }}
    {% if not loop.last %}UNION ALL{% endif %}
    {% endfor %}
)
{{ hubspot_staging_tail('src') }}
