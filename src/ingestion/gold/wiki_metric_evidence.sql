{{ metric_evidence_table() }}

-- Resolution happens HERE, once per gold build: evidence carries BOTH keys —
-- `entity_id` is the canonical person id (or '' when identity does not know
-- the email: those rows stay for coverage but reach no serving relation), and
-- `source_entity_id` keeps the source-native email for provenance. Everything
-- downstream (observations, cohorts, coverage, drilldown) reads THIS snapshot,
-- so one identity mapping answers for the whole build.
SELECT
    src.tenant_id,
    src.source_key,
    src.entity_type,
    -- Null-proof under EITHER join_use_nulls setting (models differ): the
    -- condition is non-Nullable via coalesce, and person_id is read only on
    -- the matched branch, so entity_id is a plain String fit for the sort key.
    if(
        coalesce(identity_map.email, '') != '',
        toString(assumeNotNull(identity_map.person_id)),
        ''
    ) AS entity_id,
    src.entity_id AS source_entity_id,
    src.metric_date,
    src.observed_at,
    src.measure_key,
    -- Account-qualified: several source-day record_ids (date:measure:dims
    -- hash) are identical across one person's accounts once entity_id is
    -- canonical, and both the evidence uniqueness grain and the drilldown
    -- cursor need one row per record key. Hashed, not the raw email — the id
    -- reaches the client and stays opaque.
    concat(src.record_id, ':', hex(sipHash64(src.entity_id))) AS record_id,
    src.record_kind,
    src.granularity,
    src.record_label,
    src.contribution,
    src.subject_key,
    src.dimensions,
    src.details
FROM (


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
) AS src
{{ resolved_person_id_join('src') }}
