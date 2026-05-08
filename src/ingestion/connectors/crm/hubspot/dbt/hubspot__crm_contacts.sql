{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by='(unique_key)',
    settings={'allow_nullable_key': 1},
    tags=['hubspot', 'silver:class_crm_contacts']
) }}

-- Live (`contacts`) and archived (`contacts_archived`) are sibling Bronze tables;
-- ReplacingMergeTree on `unique_key` dedups, with `_version = greatest(updatedAt, archivedAt)`
-- so an archive event always outranks the prior live update.
WITH src AS (
    {% for tbl in ['contacts', 'contacts_archived'] %}
    SELECT
        tenant_id,
        source_id,
        unique_key,
        id                                              AS contact_id,
        properties_email                                AS email,
        properties_firstname                            AS first_name,
        properties_lastname                             AS last_name,
        properties_hubspot_owner_id                     AS owner_id,
        nullIf(arrayElement(
            JSONExtract(coalesce(associations_companies, '[]'), 'Array(String)'), 1
        ), '')                                          AS account_id,
        properties_lifecyclestage                       AS lifecycle_stage,
        toJSONString(map(
            'phone',            coalesce(toString(properties_phone), ''),
            'city',             coalesce(toString(properties_city), ''),
            'state',            coalesce(toString(properties_state), ''),
            'country',          coalesce(toString(properties_country), ''),
            'jobtitle',         coalesce(toString(properties_jobtitle), ''),
            'hs_lead_status',   coalesce(toString(properties_hs_lead_status), ''),
            'hs_analytics_source', coalesce(toString(properties_hs_analytics_source), ''),
            'archived',         toString(coalesce(archived, false))
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
