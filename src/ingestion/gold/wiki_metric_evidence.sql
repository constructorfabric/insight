{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['tenant_id', 'source_key', 'measure_key', 'entity_id', 'metric_date', 'record_id'],
    schema='insight',
    alias='wiki_metric_evidence',
    tags=['gold'],
    query_settings={
        'max_memory_usage': 1610612736,
        'max_threads': 4,
        'max_bytes_before_external_group_by': 805306368,
        'max_bytes_before_external_sort': 805306368
    }
) }}


WITH
pages AS (
    SELECT
        tenant_id,
        source_id,
        page_id,
        lower(author_email) AS entity_id
    FROM {{ ref('class_wiki_pages') }} FINAL
    WHERE author_email LIKE '%@%'
),
page_creations AS (
    SELECT
        tenant_id,
        source_id,
        page_id,
        title,
        lower(author_email) AS entity_id,
        toDate(created_at) AS metric_date,
        created_at AS observed_at,
        CAST([] AS Array(Tuple(key String, value String, label Nullable(String)))) AS no_dimensions
    FROM {{ ref('class_wiki_pages') }} FINAL
    WHERE author_email LIKE '%@%'
      AND created_at IS NOT NULL
      AND page_id IS NOT NULL
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
    concat(
        toString(metric_date),
        ':',
        measure_key,
        ':',
        hex(sipHash128(toString(arrayMap(d -> tuple(d.1, d.2), dimensions))))
    ) AS record_id,
    measure_key AS record_kind,
    'source_summary' AS granularity,
    replaceAll(measure_key, '_', ' ') AS record_label,
    value AS contribution,
    CAST(NULL AS Nullable(String)) AS subject_key,
    dimensions,
    CAST(map() AS Map(String, String)) AS details
FROM value_measures
WHERE tenant_id IS NOT NULL
  AND entity_id IS NOT NULL
  AND metric_date IS NOT NULL

UNION ALL

SELECT
    assumeNotNull(tenant_id) AS tenant_id,
    'wiki' AS source_key,
    'person' AS entity_type,
    assumeNotNull(entity_id) AS entity_id,
    assumeNotNull(metric_date) AS metric_date,
    toNullable(toDateTime64(observed_at, 3)) AS observed_at,
    'pages_created' AS measure_key,
    concat(coalesce(toString(source_id), ''), ':', assumeNotNull(page_id), ':pages_created') AS record_id,
    'page' AS record_kind,
    'event' AS granularity,
    if(coalesce(title, '') = '', assumeNotNull(page_id), assumeNotNull(title)) AS record_label,
    toNullable(toFloat64(1)) AS contribution,
    CAST(NULL AS Nullable(String)) AS subject_key,
    no_dimensions AS dimensions,
    map(
        'ref', assumeNotNull(page_id),
        'title', coalesce(title, '')
    ) AS details
FROM page_creations
WHERE tenant_id IS NOT NULL
