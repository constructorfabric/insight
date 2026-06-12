-- depends_on: {{ ref('outline__bronze_promoted') }}
-- Bronze → Silver step 1: Outline documents → class_wiki_pages
--
-- One row per (tenant, source, document). Dedupe by taking the latest
-- extraction per unique_key via QUALIFY row_number() (ReplacingMergeTree
-- parts may not yet be merged at read time).
--
-- Identity resolution: emails come from the connector's own wiki_users
-- stream (users.list is the only Outline endpoint that exposes emails —
-- embedded `createdBy`/`updatedBy` objects omit the email field on
-- self-hosted instances, verified on wiki.constr.dev 2026-06-12). The
-- bronze author_email/last_editor_email columns are kept as a first
-- choice via COALESCE in case a cloud instance does embed them. No
-- cross-connector JOIN is needed — Silver Step 2 (Identity Resolution)
-- maps emails to person_id.
--
-- Column order MUST match confluence__wiki_pages exactly: class_wiki_pages
-- unions all `silver:class_wiki_pages` staging models positionally
-- (union_by_tag emits `SELECT * ... UNION ALL`).
--
-- Scaling note: materialized as view with a LEFT JOIN to spaces. Fine for
-- MVP (thousands of documents). Promote to materialized='table' or
-- 'incremental' once wiki_pages crosses ~100K rows per tenant.
{{ config(
    materialized='view',
    schema='staging',
    tags=['outline', 'silver:class_wiki_pages']
) }}

WITH pages AS (
    SELECT
        tenant_id,
        source_id,
        unique_key,
        page_id,
        -- Outline connector.yaml emits '' when a field is absent from the
        -- API response (collectionId for drafts, parentDocumentId, emails).
        -- Normalise to NULL so downstream "IS NULL" filters behave correctly
        -- and empty identifiers never reach Silver Step 2.
        nullIf(space_id, '')                                                AS space_id,
        title,
        status,
        nullIf(author_id, '')                                               AS author_id,
        nullIf(author_email, '')                                            AS author_email,
        nullIf(last_editor_id, '')                                          AS last_editor_id,
        nullIf(last_editor_email, '')                                       AS last_editor_email,
        nullIf(parent_page_id, '')                                          AS parent_page_id,
        toUInt32(coalesce(version_number, 0))                               AS version_count,
        parseDateTime64BestEffortOrNull(coalesce(created_at, ''), 3)        AS created_at,
        parseDateTime64BestEffortOrNull(coalesce(updated_at, ''), 3)        AS updated_at,
        parseDateTime64BestEffortOrNull(coalesce(collected_at, ''), 3)      AS collected_at,
        toUnixTimestamp64Milli(_airbyte_extracted_at)                       AS _version
    FROM {{ source('bronze_outline', 'wiki_pages') }}
    QUALIFY row_number() OVER (PARTITION BY unique_key ORDER BY _airbyte_extracted_at DESC) = 1
),

spaces AS (
    SELECT
        tenant_id,
        source_id,
        space_id,
        name                                                                AS space_name,
        url                                                                 AS space_url
    FROM {{ source('bronze_outline', 'wiki_spaces') }}
    QUALIFY row_number() OVER (PARTITION BY unique_key ORDER BY _airbyte_extracted_at DESC) = 1
),

users AS (
    SELECT
        tenant_id,
        source_id,
        user_id,
        lower(trim(email))                                                  AS email
    FROM {{ source('bronze_outline', 'wiki_users') }}
    WHERE email IS NOT NULL AND trim(email) != ''
    QUALIFY row_number() OVER (PARTITION BY unique_key ORDER BY _airbyte_extracted_at DESC) = 1
)

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
    coalesce(p.author_email, ua.email)                                      AS author_email,
    p.last_editor_id                                                        AS last_editor_id,
    coalesce(p.last_editor_email, ue.email)                                 AS last_editor_email,
    p.parent_page_id                                                        AS parent_page_id,
    p.version_count                                                         AS version_count,
    p.created_at                                                            AS created_at,
    p.updated_at                                                            AS updated_at,
    s.space_url                                                             AS space_url,
    'outline'                                                               AS source,
    'insight_outline'                                                       AS data_source,
    p.collected_at                                                          AS collected_at,
    p._version                                                              AS _version
FROM pages p
LEFT JOIN spaces s
    ON p.tenant_id = s.tenant_id
   AND p.source_id = s.source_id
   AND p.space_id  = s.space_id
LEFT JOIN users ua
    ON p.tenant_id = ua.tenant_id
   AND p.source_id = ua.source_id
   AND p.author_id = ua.user_id
LEFT JOIN users ue
    ON p.tenant_id      = ue.tenant_id
   AND p.source_id      = ue.source_id
   AND p.last_editor_id = ue.user_id
