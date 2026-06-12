-- depends_on: {{ ref('outline__bronze_promoted') }}
-- Bronze → Silver: Outline comments → class_wiki_engagement.
--
-- Grain: one row per (tenant, source, page, day) summarising comment
-- activity ON THAT PAGE on that day. Counterpart of
-- confluence__wiki_engagement (#285) for Outline.
--
-- Source: a single bronze table (wiki_comments) — Outline has one threaded
-- comment model instead of Confluence's 2x2 footer/inline × top/replies
-- matrix. The Confluence dimensions are derived per row:
--   - `is_reply`  = parent_comment_id is set
--   - `is_inline` = anchor_text is non-empty (comment anchored to a text
--     highlight; un-anchored comments map to Confluence "footer" semantics)
--
-- Derived metrics (same shape and order as confluence__wiki_engagement —
-- class_wiki_engagement unions staging models positionally):
--   total_comments          all comments
--   footer_comments         top-level, not anchored
--   inline_comments         top-level, anchored
--   replies                 parent_comment_id set
--   unique_commenters       distinct author_id
--   unresolved_inline_count countIf(anchored AND top-level AND resolution_status='open')
--
-- Notes:
--   - `unique_key` (post-aggregate) = `{tenant}-{source}-{page_id}-{day}`
--     plain dash-concat per ADR-0004, same as the Confluence sibling.
--   - Author email resolution is intentionally NOT done here — engagement
--     is a per-page metric, not per-person.
--   - Materialized as a view (cheap recompute); promote to incremental
--     keyed on (page_id, day) if comment volume grows past ~1M rows.
{{ config(
    materialized='view',
    schema='staging',
    tags=['outline', 'silver:class_wiki_engagement']
) }}

WITH comments AS (
    SELECT
        tenant_id,
        source_id,
        page_id,
        comment_id,
        author_id,
        parseDateTime64BestEffortOrNull(coalesce(created_at, ''), 3)   AS created_ts,
        resolution_status,
        if(coalesce(parent_comment_id, '') != '', 1, 0)                 AS is_reply,
        if(coalesce(anchor_text, '') != '', 1, 0)                       AS is_inline,
        _airbyte_extracted_at                                           AS extracted_at
    FROM {{ source('bronze_outline', 'wiki_comments') }}
    WHERE author_id IS NOT NULL AND author_id != ''
    QUALIFY row_number() OVER (PARTITION BY unique_key ORDER BY _airbyte_extracted_at DESC) = 1
)

SELECT
    tenant_id,
    source_id,
    CAST(concat(
        coalesce(tenant_id, ''), '-',
        coalesce(source_id, ''), '-',
        coalesce(page_id, ''), '-',
        toString(toDate(created_ts))
    ) AS String)                                            AS unique_key,
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
    'outline'                                               AS source,
    'insight_outline'                                       AS data_source,
    CAST(max(extracted_at) AS Nullable(DateTime64(3)))      AS collected_at,
    toUnixTimestamp64Milli(max(extracted_at))               AS _version
FROM comments
WHERE created_ts IS NOT NULL
  -- Defensive: drop rows where page_id failed to populate from the API
  -- response. comments.list always returns documentId in our testing, so
  -- this filter is expected to be a no-op, but without it a malformed
  -- bronze row would group into a fake (tenant, source, '', day) bucket.
  AND page_id IS NOT NULL AND page_id != ''
GROUP BY tenant_id, source_id, page_id, toDate(created_ts)
