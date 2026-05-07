-- Bronze → Silver: Confluence comments → class_wiki_engagement.
--
-- Grain: one row per (tenant, source, page, day) summarising comment
-- activity ON THAT PAGE on that day across all comment kinds. Feeds
-- "engagement vs ignored" page-level dashboards (issue #285).
--
-- AC#2 waiver note (issue #285): the issue's literal AC#2 asked for
-- per-event comment rows in `class_wiki_activity` keyed on `person_key`.
-- We deliberately deliver a per-page-day class instead — see the
-- design-note block on `class_wiki_engagement` in
-- `src/ingestion/silver/wiki/schema.yml` for the rationale.
--
-- Sources (4 bronze tables, all populated by the comments substreams
-- added in PR for #285):
--   wiki_footer_comments         top-level page-bottom comments
--   wiki_footer_comment_replies  replies to footer comments
--   wiki_inline_comments         top-level highlight-anchored comments
--   wiki_inline_comment_replies  replies to inline comments
--
-- Each row is shaped before UNION:
--   - `is_reply` = 1 if from a *_replies table
--   - `is_inline` = 1 if from a wiki_inline_*  table
--   - `day` = toDate(created_at) — Confluence timestamps are UTC
--   - `unique_key` (post-aggregate) = MD5({tenant}-{source}-{page}-{day})
--     so the silver table can dedup via ReplacingMergeTree
--
-- Derived metrics:
--   total_comments          all 4 streams combined
--   footer_comments         top-level footer (non-reply)
--   inline_comments         top-level inline (non-reply)
--   replies                 footer + inline replies
--   unique_commenters       distinct author_id across all 4 streams
--   unresolved_inline_count countIf(inline AND resolution_status='open' AND NOT is_reply)
--
-- Notes:
--   - Per ADR-0001 / ADR-0004 conventions: this model emits a `_version`
--     column (toUnixTimestamp64Milli of latest extraction) so the silver
--     RMT can deduplicate by `unique_key`. New comments on an existing
--     (page, day) bump `_version` and the silver merge picks them up.
--   - Author email resolution is intentionally NOT done here — engagement
--     is a per-page metric, not per-person. Drilldown to "who commented"
--     happens via the bronze tables or a future per-person staging model.
--   - Materialized as a view (cheap recompute); promote to incremental
--     keyed on (page_id, day) if comment volume grows past ~1M rows.

{{ config(
    materialized='view',
    schema='staging',
    tags=['confluence', 'silver:class_wiki_engagement']
) }}

WITH comments AS (
    -- footer top-level
    SELECT
        tenant_id, source_id, page_id, comment_id, author_id,
        parseDateTime64BestEffortOrNull(coalesce(created_at, ''), 3) AS created_ts,
        resolution_status,
        0 AS is_reply,
        0 AS is_inline,
        _airbyte_extracted_at AS extracted_at
    FROM {{ source('bronze_confluence', 'wiki_footer_comments') }}
    WHERE author_id IS NOT NULL AND author_id != ''
    QUALIFY row_number() OVER (PARTITION BY unique_key ORDER BY _airbyte_extracted_at DESC) = 1

    UNION ALL

    -- footer replies
    SELECT
        tenant_id, source_id, page_id, comment_id, author_id,
        parseDateTime64BestEffortOrNull(coalesce(created_at, ''), 3) AS created_ts,
        CAST(NULL AS Nullable(String)) AS resolution_status,
        1 AS is_reply,
        0 AS is_inline,
        _airbyte_extracted_at AS extracted_at
    FROM {{ source('bronze_confluence', 'wiki_footer_comment_replies') }}
    WHERE author_id IS NOT NULL AND author_id != ''
    QUALIFY row_number() OVER (PARTITION BY unique_key ORDER BY _airbyte_extracted_at DESC) = 1

    UNION ALL

    -- inline top-level
    SELECT
        tenant_id, source_id, page_id, comment_id, author_id,
        parseDateTime64BestEffortOrNull(coalesce(created_at, ''), 3) AS created_ts,
        resolution_status,
        0 AS is_reply,
        1 AS is_inline,
        _airbyte_extracted_at AS extracted_at
    FROM {{ source('bronze_confluence', 'wiki_inline_comments') }}
    WHERE author_id IS NOT NULL AND author_id != ''
    QUALIFY row_number() OVER (PARTITION BY unique_key ORDER BY _airbyte_extracted_at DESC) = 1

    UNION ALL

    -- inline replies
    SELECT
        tenant_id, source_id, page_id, comment_id, author_id,
        parseDateTime64BestEffortOrNull(coalesce(created_at, ''), 3) AS created_ts,
        CAST(NULL AS Nullable(String)) AS resolution_status,
        1 AS is_reply,
        1 AS is_inline,
        _airbyte_extracted_at AS extracted_at
    FROM {{ source('bronze_confluence', 'wiki_inline_comment_replies') }}
    WHERE author_id IS NOT NULL AND author_id != ''
    QUALIFY row_number() OVER (PARTITION BY unique_key ORDER BY _airbyte_extracted_at DESC) = 1
)

SELECT
    tenant_id,
    source_id,
    CAST(MD5(concat(
        coalesce(tenant_id, ''), '-',
        coalesce(source_id, ''), '-',
        coalesce(page_id, ''), '-',
        toString(toDate(created_ts))
    )) AS FixedString(16))                                  AS unique_key,
    page_id,
    toDate(created_ts)                                      AS day,
    -- counts
    toUInt32(count())                                       AS total_comments,
    toUInt32(countIf(is_inline = 0 AND is_reply = 0))       AS footer_comments,
    toUInt32(countIf(is_inline = 1 AND is_reply = 0))       AS inline_comments,
    toUInt32(countIf(is_reply = 1))                         AS replies,
    toUInt32(uniqExact(author_id))                          AS unique_commenters,
    toUInt32(countIf(is_inline = 1 AND is_reply = 0 AND resolution_status = 'open'))
                                                            AS unresolved_inline_count,
    -- envelope
    'confluence'                                            AS source,
    'insight_confluence'                                    AS data_source,
    CAST(max(extracted_at) AS Nullable(DateTime64(3)))      AS collected_at,
    toUnixTimestamp64Milli(max(extracted_at))               AS _version
FROM comments
WHERE created_ts IS NOT NULL
  -- Defensive: drop rows where page_id failed to populate from the API
  -- response. The v2 listing and /children endpoints always return pageId
  -- in our integration tests, so this filter is expected to be a no-op,
  -- but without it a malformed bronze row would group into a fake
  -- (tenant, source, '', day) engagement bucket.
  AND page_id IS NOT NULL AND page_id != ''
GROUP BY tenant_id, source_id, page_id, toDate(created_ts)
