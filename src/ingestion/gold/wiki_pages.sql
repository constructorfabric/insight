{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['tenant_id', 'author_email', 'created_at'],
    partition_by='toYYYYMM(created_date)',
    schema=var('gold_database'),
    settings={'allow_nullable_key': 1},
    tags=['gold'],
    query_settings=metric_serving_query_settings()
) }}

-- Authored wiki pages: one row per page a person created, with the space it
-- lives in and the day the source dated its creation.
--
-- INVARIANT: identity is the author email the source recorded, normalized and
-- nothing more; the person is bound when a query runs.

SELECT
    -- SAFETY: both are safe under the WHERE below.
    assumeNotNull(pages.tenant_id) AS tenant_id,
    pages.source_id AS source_id,
    assumeNotNull(pages.page_id) AS page_id,
    pages.title AS title,
    pages.space_name AS space_name,
    assumeNotNull(lower(trimBoth(pages.author_email))) AS author_email,
    -- A source that dates no creation still described a page; the row stays and
    -- falls outside every period until the source dates it.
    pages.created_at AS created_at,
    toDate(pages.created_at) AS created_date
FROM {{ ref('class_wiki_pages') }} AS pages FINAL
WHERE pages.tenant_id IS NOT NULL
  AND pages.page_id IS NOT NULL
  AND pages.author_email LIKE '%@%'
