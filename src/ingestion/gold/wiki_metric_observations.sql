{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['source_key', 'measure_key', 'entity_id', 'metric_date'],
    schema='insight',
    alias='wiki_metric_observations',
    tags=['gold'],
    query_settings={
        'max_memory_usage': 3221225472,
        'max_threads': 4,
        'max_bytes_before_external_group_by': 805306368,
        'max_bytes_before_external_sort': 805306368,
        'join_algorithm': 'grace_hash,hash'
    }
) }}

-- Source measure observations for the unified metrics runtime, wiki family.
-- Reads the wiki class contracts only (class_wiki_pages, class_wiki_activity,
-- class_wiki_engagement); every measure is emitted through the shape macros
-- in macros/metric_observation_measures.sql. No dimensions: wiki sources
-- (Confluence, Outline) feed one undifferentiated family.
--
-- Materialized as a sorted table: the pipeline runs once per dbt build —
-- the only time the silver inputs can have changed — and the ordering key
-- mirrors the runtime's filter shape (source_key, measure_key, entity_id,
-- metric_date), so single-measure queries read index-pruned ranges.
--
-- Grain per measure:
--   day-grain sums (creation date, page author):   pages_created (1/page —
--                    the page object's own author/created_at are the
--                    canonical creation facts; a version-derived proxy
--                    undercounts imported pages)
--   day-grain sums (edit date, version author):    edits (logical edit
--                    sessions — autosave bursts collapsed in silver, see
--                    class_wiki_activity), pages_edited (distinct pages
--                    touched that day)
--   day-grain sums (comment date, page author):    comments (engagement
--                    RECEIVED on the person's pages — footer + inline +
--                    replies; the commenter is deliberately not the entity,
--                    see class_wiki_engagement's page-centric design note)
--
-- Attribution: entity_id = lower(author_email); only email-shaped keys pass.
-- Confluence resolves emails through the Jira directory join in staging and
-- yields NULL on tenants without Jira — those rows are excluded as
-- unmatchable rather than carried as dead entities (cohorts and API requests
-- address people by email). Outline resolves from its own user stream.
--
-- Memory shape: every class read keeps FINAL (ReplacingMergeTree dedup) over
-- a pruned column set. class_wiki_activity is already (author, day) grain,
-- so its sums are near-free. The single join in the model attributes
-- page-day comment rollups to the page author: engagement ⋈ pages on
-- (tenant_id, source_id, page_id) — source_id is part of the key so a
-- page_id colliding across two wiki instances of one tenant cannot fan out.
--
-- Peer measurability (who enters a metric's peer pool) is decided HERE, by
-- row emission — the runtime never fabricates zeros. All four measures are
-- engagement-gated: a row exists only where the source recorded authorship,
-- editing, or received comments. Rostered-but-inactive people take no
-- standing rather than dragging peer medians toward zero.

WITH
-- Page -> author attribution for the comment join. No created_at gate:
-- comment attribution needs only the page's author, so a page snapshot
-- without a creation timestamp still receives its comments.
pages AS (
    SELECT
        tenant_id,
        source_id,
        page_id,
        lower(author_email) AS entity_id
    FROM {{ ref('class_wiki_pages') }} FINAL
    WHERE author_email LIKE '%@%'
),
-- Dated branch for pages_created; the creation date is the metric date, so
-- only here does a missing created_at exclude the page.
page_creations AS (
    SELECT
        tenant_id,
        lower(author_email) AS entity_id,
        toDate(created_at) AS metric_date,
        CAST([] AS Array(Tuple(key String, value String, label Nullable(String)))) AS no_dimensions
    FROM {{ ref('class_wiki_pages') }} FINAL
    WHERE author_email LIKE '%@%'
      AND created_at IS NOT NULL
),
activity AS (
    SELECT
        tenant_id,
        lower(author_email) AS entity_id,
        day AS metric_date,
        total_edits,
        pages_edited,
        CAST([] AS Array(Tuple(key String, value String, label Nullable(String)))) AS no_dimensions
    FROM {{ ref('class_wiki_activity') }} FINAL
    WHERE author_email LIKE '%@%'
      AND day IS NOT NULL
),
engagement AS (
    SELECT
        e.tenant_id AS tenant_id,
        p.entity_id AS entity_id,
        e.day AS metric_date,
        e.total_comments AS total_comments,
        CAST([] AS Array(Tuple(key String, value String, label Nullable(String)))) AS no_dimensions
    FROM (
        SELECT
            tenant_id,
            source_id,
            page_id,
            day,
            total_comments
        FROM {{ ref('class_wiki_engagement') }} FINAL
        WHERE day IS NOT NULL
    ) AS e
    INNER JOIN pages AS p
        ON e.tenant_id = p.tenant_id
       AND e.source_id = p.source_id
       AND e.page_id = p.page_id
),
value_measures AS (
    {{ sum_measure('pages_created', 'page_creations', '1', 'no_dimensions') }}

    UNION ALL

    {{ sum_measure('edits', 'activity', 'total_edits', 'no_dimensions') }}

    UNION ALL

    {{ sum_measure('pages_edited', 'activity', 'pages_edited', 'no_dimensions') }}

    UNION ALL

    {{ sum_measure('comments', 'engagement', 'total_comments', 'no_dimensions') }}
)
SELECT
    assumeNotNull(tenant_id) AS tenant_id,
    'wiki' AS source_key,
    'person' AS entity_type,
    assumeNotNull(entity_id) AS entity_id,
    assumeNotNull(metric_date) AS metric_date,
    CAST(NULL AS Nullable(DateTime64(3))) AS observed_at,
    measure_key,
    value,
    CAST(NULL AS Nullable(String)) AS subject_key,
    dimensions
FROM value_measures
WHERE tenant_id IS NOT NULL
  AND entity_id IS NOT NULL
  AND metric_date IS NOT NULL
