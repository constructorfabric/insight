-- depends_on: {{ ref('confluence__bronze_promoted') }}
-- Bronze → Silver step 1: Confluence pages → class_wiki_pages
--
-- One row per (tenant, source, page). Dedupe by taking the latest extraction
-- per unique_key via QUALIFY row_number() (ReplacingMergeTree parts may not
-- yet be merged at read time).
--
-- Identity resolution: Confluence v2 API returns `authorId` (Atlassian
-- accountId) but not email. Resolved in Silver Step 1 via LEFT JOIN with
-- bronze_jira.jira_user on the same accountId namespace. If Jira is not
-- provisioned for the tenant, the JOIN is skipped at compile-time and
-- author_email / last_editor_email fall back to NULL — Silver Step 2
-- (Identity Resolution) maps author_id → person_id directly.
--
-- Scaling note: materialized as view with LEFT JOINs to spaces and users.
-- Fine for MVP (thousands of pages). Promote to materialized='table' or
-- 'incremental' once wiki_pages crosses ~100K rows per tenant.
{{ config(
    materialized='view',
    schema='staging',
    tags=['confluence', 'silver:class_wiki_pages']
) }}

{%- set jira_user = adapter.get_relation(database=none, schema='bronze_jira', identifier='jira_user') -%}

WITH pages AS (
    SELECT
        tenant_id,
        source_id,
        unique_key,
        page_id,
        space_id,
        title,
        status,
        -- Confluence connector.yaml emits '' when a field is absent from
        -- the API response (authorId, parentId, etc). Normalise to NULL so
        -- downstream "IS NULL" filters (e.g. top-level pages with no parent)
        -- behave correctly and empty identifiers never reach Silver Step 2.
        nullIf(author_id, '')                                               AS author_id,
        nullIf(last_editor_id, '')                                          AS last_editor_id,
        nullIf(parent_page_id, '')                                          AS parent_page_id,
        toUInt32(coalesce(version_number, 0))                               AS version_count,
        parseDateTime64BestEffortOrNull(coalesce(created_at, ''), 3)        AS created_at,
        parseDateTime64BestEffortOrNull(coalesce(updated_at, ''), 3)        AS updated_at,
        parseDateTime64BestEffortOrNull(coalesce(collected_at, ''), 3)      AS collected_at,
        toUnixTimestamp64Milli(_airbyte_extracted_at)                       AS _version
    FROM {{ source('bronze_confluence', 'wiki_pages') }}
    QUALIFY row_number() OVER (PARTITION BY unique_key ORDER BY _airbyte_extracted_at DESC) = 1
),

spaces AS (
    SELECT
        tenant_id,
        source_id,
        space_id,
        name                                                                AS space_name,
        url                                                                 AS space_url
    FROM {{ source('bronze_confluence', 'wiki_spaces') }}
    QUALIFY row_number() OVER (PARTITION BY unique_key ORDER BY _airbyte_extracted_at DESC) = 1
)

{%- if jira_user %}
, users AS (
    SELECT
        tenant_id,
        account_id,
        lower(trim(email))                                                  AS email
    FROM {{ source('bronze_jira', 'jira_user') }}
    WHERE email IS NOT NULL AND trim(email) != ''
    QUALIFY row_number() OVER (PARTITION BY tenant_id, account_id ORDER BY _airbyte_extracted_at DESC) = 1
)
{%- endif %}

-- Explicit `AS column_name` aliases are required: ClickHouse keeps the
-- table-alias prefix in the projected column name otherwise (e.g. column
-- becomes `p.tenant_id` instead of `tenant_id`), which breaks downstream
-- consumers and `not_null`/`unique` tests.
SELECT
    p.tenant_id                                                             AS tenant_id,
    p.source_id                                                             AS source_id,
    p.unique_key                                                            AS unique_key,
    p.page_id                                                               AS page_id,
    p.space_id                                                              AS space_id,
    s.space_name                                                            AS space_name,
    p.title                                                                 AS title,
    p.status                                                                AS status,
    p.author_id                                                             AS author_id,
    {% if jira_user %}ua.email{% else %}CAST(NULL AS Nullable(String)){% endif %}                                                    AS author_email,
    p.last_editor_id                                                        AS last_editor_id,
    {% if jira_user %}ue.email{% else %}CAST(NULL AS Nullable(String)){% endif %}                                                    AS last_editor_email,
    p.parent_page_id                                                        AS parent_page_id,
    p.version_count                                                         AS version_count,
    p.created_at                                                            AS created_at,
    p.updated_at                                                            AS updated_at,
    s.space_url                                                             AS space_url,
    'confluence'                                                            AS source,
    'insight_confluence'                                                    AS data_source,
    p.collected_at                                                          AS collected_at,
    p._version                                                              AS _version
FROM pages p
LEFT JOIN spaces s
    ON p.tenant_id = s.tenant_id
   AND p.source_id = s.source_id
   AND p.space_id  = s.space_id
{%- if jira_user %}
LEFT JOIN users ua
    ON p.tenant_id = ua.tenant_id
   AND p.author_id = ua.account_id
LEFT JOIN users ue
    ON p.tenant_id = ue.tenant_id
   AND p.last_editor_id = ue.account_id
{%- endif %}
