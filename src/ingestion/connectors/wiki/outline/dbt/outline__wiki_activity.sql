-- depends_on: {{ ref('outline__bronze_promoted') }}
-- Bronze → Silver step 1: Outline revisions → class_wiki_activity
--
-- Per-user per-day edit activity rolled up from wiki_page_versions. One row
-- per (tenant, source, author, day) with counts of pages edited, total edits
-- (sessions, see below), pages created.
--
-- Edit-session collapse (same policy as confluence__wiki_activity, #259):
-- Outline creates a new revision on every autosave debounce, so counting raw
-- revisions overstates editing activity. Consecutive revisions of the same
-- (tenant, source, page, author) are grouped into sessions: a new session
-- starts when the gap to the previous revision exceeds `session_gap_seconds`
-- (default 30 min). `total_edits` counts distinct (page, session) pairs.
--
-- Revision ordinals: the Outline API has no per-revision version number
-- (revisions are UUIDs). The ordinal is derived via row_number() over
-- created_at per document; `pages_created` counts revisions with ordinal 1
-- (the document's first revision), mirroring Confluence's version_number = 1.
-- The ordinal is computed BEFORE the author filter so an authorless first
-- revision does not promote the second revision's author to "creator".
--
-- Identity resolution: emails come from the connector's own wiki_users
-- stream — revisions.list embeds a `createdBy` user object but WITHOUT the
-- email field (verified on wiki.constr.dev 2026-06-12), so the bronze
-- author_email is normally empty and the wiki_users JOIN is the real
-- resolver (COALESCE keeps the embedded value first in case a cloud
-- instance provides it). No cross-connector JOIN needed.
--
-- Column order MUST match confluence__wiki_activity exactly (positional
-- UNION ALL in union_by_tag).
--
-- Scaling note: materialized as view. Fine for MVP (tens of thousands of
-- revisions). Promote to materialized='incremental' keyed on
-- (author_id, day) once wiki_page_versions grows past ~1M rows.
{{ config(
    materialized='view',
    schema='staging',
    tags=['outline', 'silver:class_wiki_activity']
) }}

{# Session-collapse threshold: gap between two consecutive revisions of the
   same (page, author) above which a new edit session is started. 30 min
   absorbs autosave bursts but keeps morning-vs-evening edits separate. #}
{%- set session_gap_seconds = 1800 -%}

WITH revisions AS (
    SELECT
        tenant_id,
        source_id,
        page_id,
        nullIf(author_id, '')                                                 AS author_id,
        nullIf(author_email, '')                                              AS author_email,
        parseDateTime64BestEffortOrNull(coalesce(created_at, ''), 3)         AS created_at_ts,
        toDate(parseDateTime64BestEffortOrNull(coalesce(created_at, ''), 3)) AS day,
        parseDateTime64BestEffortOrNull(coalesce(collected_at, ''), 3)       AS collected_at,
        _airbyte_extracted_at                                                 AS extracted_at
    FROM {{ source('bronze_outline', 'wiki_page_versions') }}
    QUALIFY row_number() OVER (PARTITION BY unique_key ORDER BY _airbyte_extracted_at DESC) = 1
),

revisions_with_ordinal AS (
    -- Per-document revision ordinal (1 = the revision that created the
    -- document). Computed before the author filter — see header comment.
    SELECT
        *,
        row_number() OVER (
            PARTITION BY tenant_id, source_id, page_id
            ORDER BY created_at_ts
        ) AS revision_ordinal
    FROM revisions
    WHERE created_at_ts IS NOT NULL
),

revisions_with_gap AS (
    -- gap_seconds = seconds since previous revision of the SAME (page, author).
    -- NULL for the first revision of each (page, author) chain.
    SELECT
        *,
        dateDiff(
            'second',
            lagInFrame(created_at_ts) OVER (
                PARTITION BY tenant_id, source_id, page_id, author_id
                ORDER BY created_at_ts
            ),
            created_at_ts
        ) AS gap_seconds
    FROM revisions_with_ordinal
    WHERE author_id IS NOT NULL
),

revisions_with_session AS (
    -- session_id is a running counter within (page, author). Increments on
    -- the first revision of the chain (gap IS NULL) and on every gap > threshold.
    -- Not globally unique — only unique within (page, author) — but downstream
    -- uniqExact((page_id, session_id)) makes it work as a per-group session key.
    SELECT
        *,
        sum(CASE WHEN gap_seconds IS NULL OR gap_seconds > {{ session_gap_seconds }} THEN 1 ELSE 0 END)
            OVER (
                PARTITION BY tenant_id, source_id, page_id, author_id
                ORDER BY created_at_ts
            ) AS session_id
    FROM revisions_with_gap
),

agg AS (
    SELECT
        tenant_id,
        source_id,
        author_id,
        -- Embedded email per (author, day): latest revision wins. Normally
        -- empty (see header) — the wiki_users JOIN below is the resolver.
        argMax(author_email, created_at_ts)                                 AS embedded_email,
        day,
        -- uniqExact (not uniq): uniq is HyperLogLog and can miscount by a
        -- full unit for small per-day page counts, directly skewing the
        -- pages_edited metric. uniqExact is correct at this scale.
        uniqExact(page_id)                                                  AS pages_edited,
        -- One logical edit per (page, session) pair — collapses autosave
        -- bursts into a single counted edit. See header comment for rationale.
        uniqExact((page_id, session_id))                                    AS total_edits,
        countIf(revision_ordinal = 1)                                       AS pages_created,
        max(collected_at)                                                   AS collected_at_max,
        -- _version per group: latest bronze extraction. Changes only when new
        -- revisions arrive for this (author, day), so downstream silver
        -- incremental filter (`_version > max(_version)`) skips unchanged groups.
        max(extracted_at)                                                   AS extracted_at_max
    FROM revisions_with_session
    WHERE day IS NOT NULL
    GROUP BY tenant_id, source_id, author_id, day
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

SELECT
    a.tenant_id                                                             AS tenant_id,
    a.source_id                                                             AS source_id,
    CAST(concat(
        coalesce(a.tenant_id, ''), '-',
        coalesce(a.source_id, ''), '-',
        a.author_id, '-',
        toString(a.day)
    ) AS String)                                                            AS unique_key,
    a.author_id                                                             AS author_id,
    coalesce(a.embedded_email, u.email)                                     AS author_email,
    a.day                                                                   AS day,
    toUInt32(a.pages_edited)                                                AS pages_edited,
    toUInt32(a.total_edits)                                                 AS total_edits,
    toUInt32(a.pages_created)                                               AS pages_created,
    'outline'                                                               AS source,
    'insight_outline'                                                       AS data_source,
    CAST(a.collected_at_max AS Nullable(DateTime64(3)))                     AS collected_at,
    toUnixTimestamp64Milli(a.extracted_at_max)                              AS _version
FROM agg a
LEFT JOIN users u
    ON a.tenant_id = u.tenant_id
   AND a.source_id = u.source_id
   AND a.author_id = u.user_id
