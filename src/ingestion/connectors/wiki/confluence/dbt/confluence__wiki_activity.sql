-- Bronze → Silver step 1: Confluence page versions → class_wiki_activity
--
-- Per-user per-day edit activity rolled up from wiki_page_versions. One row
-- per (tenant, source, author, day) with counts of pages edited, total edits
-- (sessions, see below), pages created (version_number = 1).
--
-- Edit-session collapse (issue #259):
-- Confluence creates a new wiki_page_versions row on every save — autosaves,
-- rapid republishes, plugin-driven saves, etc. Counting raw versions
-- systematically overstates editing activity (e.g., 27 versions for a
-- single 8-hour session of editing one page). To recover "logical edit",
-- we group consecutive versions of the same (tenant, source, page, author)
-- into sessions: a new session starts when the gap to the previous version
-- of the same key exceeds `session_gap_seconds` (default 30 min). The
-- threshold is conservative — long enough to absorb autosave bursts and
-- short coffee breaks, short enough to keep a "morning vs evening" split.
-- `total_edits` then counts distinct (page, session) pairs instead of raw
-- versions. `pages_edited` and `pages_created` are unaffected by the bug
-- and stay as before.
--
-- Dropped: `minor_edits` / `major_edits` (issue #260). The Confluence Cloud
-- `/pages/{id}/versions` endpoint always returns `minorEdit: false` for all
-- versions regardless of how the flag was set on PUT — verified empirically
-- on 2026-05-05. The split conveys no information. The bronze field
-- `wiki_page_versions.minor_edit` is preserved as-is for forward
-- compatibility, but Silver ignores it.
--
-- Identity resolution: same pattern as confluence__wiki_pages — LEFT JOIN
-- with bronze_jira.jira_user on accountId. Skipped at compile-time if Jira
-- is not provisioned; author_email falls back to NULL.
--
-- Scaling note: materialized as view, which recomputes the versions → agg
-- pipeline on every downstream query. Fine for MVP (tens of thousands of
-- versions). Promote to materialized='incremental' keyed on (author_id, day)
-- once wiki_page_versions grows past ~1M rows or Gold query latency rises.
{{ config(
    materialized='view',
    schema='staging',
    tags=['confluence', 'silver:class_wiki_activity']
) }}

{%- set jira_user = adapter.get_relation(database=none, schema='bronze_jira', identifier='jira_user') -%}

{# Session-collapse threshold: gap between two consecutive versions of the
   same (page, author) above which a new edit session is started. 30 min
   absorbs autosave bursts but keeps morning-vs-evening edits separate. #}
{%- set session_gap_seconds = 1800 -%}

WITH versions AS (
    SELECT
        tenant_id,
        source_id,
        page_id,
        author_id,
        toUInt32(coalesce(version_number, 0))                                AS version_number,
        parseDateTime64BestEffortOrNull(coalesce(created_at, ''), 3)         AS created_at_ts,
        toDate(parseDateTime64BestEffortOrNull(coalesce(created_at, ''), 3)) AS day,
        parseDateTime64BestEffortOrNull(coalesce(collected_at, ''), 3)       AS collected_at,
        _airbyte_extracted_at                                                AS extracted_at
    FROM {{ source('bronze_confluence', 'wiki_page_versions') }}
    WHERE author_id IS NOT NULL AND author_id != ''
    QUALIFY row_number() OVER (PARTITION BY unique_key ORDER BY _airbyte_extracted_at DESC) = 1
),

versions_with_gap AS (
    -- gap_seconds = seconds since previous version of the SAME (page, author).
    -- NULL for the first version of each (page, author) chain.
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
    FROM versions
    WHERE created_at_ts IS NOT NULL
),

versions_with_session AS (
    -- session_id is a running counter within (page, author). Increments on
    -- the first version of the chain (gap IS NULL) and on every gap > threshold.
    -- Not globally unique — only unique within (page, author) — but downstream
    -- uniqExact((page_id, session_id)) makes it work as a per-group session key.
    SELECT
        *,
        sum(CASE WHEN gap_seconds IS NULL OR gap_seconds > {{ session_gap_seconds }} THEN 1 ELSE 0 END)
            OVER (
                PARTITION BY tenant_id, source_id, page_id, author_id
                ORDER BY created_at_ts
            ) AS session_id
    FROM versions_with_gap
),

agg AS (
    SELECT
        tenant_id,
        source_id,
        author_id,
        day,
        -- uniqExact (not uniq): uniq is HyperLogLog and can miscount by a
        -- full unit for small per-day page counts, directly skewing the
        -- pages_edited metric. uniqExact is correct at this scale.
        uniqExact(page_id)                                                  AS pages_edited,
        -- One logical edit per (page, session) pair — collapses autosave
        -- bursts and rapid republishes into a single counted edit. See
        -- header comment for rationale.
        uniqExact((page_id, session_id))                                    AS total_edits,
        countIf(version_number = 1)                                         AS pages_created,
        max(collected_at)                                                   AS collected_at_max,
        -- _version per group: latest bronze extraction. Changes only when new
        -- versions arrive for this (author, day), so downstream silver
        -- incremental filter (`_version > max(_version)`) skips unchanged groups.
        max(extracted_at)                                                   AS extracted_at_max
    FROM versions_with_session
    WHERE day IS NOT NULL
    GROUP BY tenant_id, source_id, author_id, day
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

SELECT
    a.tenant_id,
    a.source_id,
    CAST(concat(
        coalesce(a.tenant_id, ''), '-',
        coalesce(a.source_id, ''), '-',
        a.author_id, '-',
        toString(a.day)
    ) AS String)                                                            AS unique_key,
    a.author_id,
    {% if jira_user %}u.email{% else %}CAST(NULL AS Nullable(String)){% endif %}                                                     AS author_email,
    a.day,
    toUInt32(a.pages_edited)                                                AS pages_edited,
    toUInt32(a.total_edits)                                                 AS total_edits,
    toUInt32(a.pages_created)                                               AS pages_created,
    'confluence'                                                            AS source,
    'insight_confluence'                                                    AS data_source,
    CAST(a.collected_at_max AS Nullable(DateTime64(3)))                     AS collected_at,
    toUnixTimestamp64Milli(a.extracted_at_max)                              AS _version
FROM agg a
{%- if jira_user %}
LEFT JOIN users u
    ON a.tenant_id = u.tenant_id
   AND a.author_id = u.account_id
{%- endif %}
