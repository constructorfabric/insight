-- depends_on: {{ ref('jira__task_field_kind') }}
{{ config(
    materialized='table',
    schema='staging',
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    query_settings={
        'join_use_nulls': 1,
        'max_bytes_before_external_group_by': 2000000000,
        'max_bytes_before_external_sort': 2000000000,
    },
    tags=['jira', 'staging', 'silver:class_task_references']
) }}

-- One row per issue an issue points at, per place it points from: a link, a
-- field, a comment. See `class_task_references` for the contract.

WITH issues AS (
    SELECT
        tenant_id,
        CAST(source_id AS String)                                     AS insight_source_id,
        CAST(jira_id AS String)                                       AS issue_id,
        argMax(COALESCE(toString(id_readable), ''), _airbyte_extracted_at) AS current_key,
        argMax(parseDateTime64BestEffortOrNull(created, 3), _airbyte_extracted_at) AS created_at
    FROM {{ source('bronze_jira', 'jira_issue') }}
    GROUP BY tenant_id, insight_source_id, issue_id
),

-- An issue the connector does not sync is known by the key the newest snapshot
-- linking to it names.
current_keys AS (
    SELECT
        tenant_id,
        insight_source_id,
        issue_id,
        argMin(current_key, precedence)                               AS current_key
    FROM (
        SELECT tenant_id, insight_source_id, issue_id, current_key, toUInt8(1) AS precedence
        FROM issues
        WHERE current_key != ''

        UNION ALL

        SELECT tenant_id, insight_source_id, target_id AS issue_id, argMax(target_key, extracted_at) AS current_key, toUInt8(2) AS precedence
        FROM {{ ref('jira__issue_link_snapshot') }} FINAL
        WHERE target_id != '' AND target_key != ''
        GROUP BY tenant_id, insight_source_id, target_id
    )
    GROUP BY tenant_id, insight_source_id, issue_id
),

-- Every field with a value a person sets, scanned as Jira displays it.
-- Linked issues and the parent are links, read from `jira__task_links`.
scanned_fields AS (
    SELECT
        tenant_id,
        insight_source_id,
        field_id,
        field_kind,
        if(field_kind = 'long_text', 'long_text', 'short_text')       AS found_kind
    FROM {{ ref('jira__task_field_kind') }}
    WHERE field_kind NOT IN ('ignored', 'UNKNOWN', 'link_array')
      AND field_id != 'parent'
),

texts AS (
    SELECT tenant_id, insight_source_id, text_id, content_form, content
    FROM {{ ref('jira__task_field_text') }} FINAL
),

current_long_text AS (
    SELECT
        c.tenant_id                                                   AS tenant_id,
        c.insight_source_id                                           AS insight_source_id,
        c.issue_id                                                    AS issue_id,
        c.field_id                                                    AS field_id,
        CAST('long_text' AS String)                                   AS found_kind,
        t.content_form                                                AS content_form,
        t.content                                                     AS content
    FROM {{ ref('jira__task_field_current_text') }} AS c FINAL
    INNER JOIN texts AS t
        ON t.tenant_id = c.tenant_id
       AND t.insight_source_id = c.insight_source_id
       AND t.text_id = c.text_id
),

current_short_values AS (
    SELECT
        s.tenant_id                                                   AS tenant_id,
        s.insight_source_id                                           AS insight_source_id,
        s.issue_id                                                    AS issue_id,
        s.field_id                                                    AS field_id,
        f.found_kind                                                  AS found_kind,
        CAST('plain' AS String)                                       AS content_form,
        arrayStringConcat(s.value_displays, '\n')                    AS content
    FROM {{ ref('jira__issue_field_snapshot') }} AS s FINAL
    INNER JOIN scanned_fields AS f
        ON f.tenant_id = s.tenant_id
       AND f.insight_source_id = s.insight_source_id
       AND f.field_id = s.field_id
    WHERE f.field_kind != 'long_text'
),

-- Every value a field ever held, so a key edited out of a body stays visible
-- as no longer current.
past_values AS (
    SELECT
        h.tenant_id                                                   AS tenant_id,
        h.insight_source_id                                           AS insight_source_id,
        h.issue_id                                                    AS issue_id,
        h.field_id                                                    AS field_id,
        f.found_kind                                                  AS found_kind,
        toDateTime64(h.event_at, 3)                                   AS seen_at,
        if(f.field_kind = 'long_text', t.content_form, 'plain')       AS content_form,
        if(f.field_kind = 'long_text', t.content, arrayStringConcat(h.value_displays, '\n')) AS content
    FROM {{ ref('jira__field_history_derived') }} AS h FINAL
    INNER JOIN scanned_fields AS f
        ON f.tenant_id = h.tenant_id
       AND f.insight_source_id = h.insight_source_id
       AND f.field_id = h.field_id
    LEFT JOIN texts AS t
        ON t.tenant_id = h.tenant_id
       AND t.insight_source_id = h.insight_source_id
       AND t.text_id = h.value_ids[1]
    WHERE notEmpty(h.value_ids)
),

field_values AS (
    SELECT tenant_id, insight_source_id, issue_id, field_id, found_kind, CAST(NULL AS Nullable(DateTime64(3))) AS seen_at,
           toUInt8(1) AS is_current, content_form, content
    FROM current_long_text

    UNION ALL

    SELECT tenant_id, insight_source_id, issue_id, field_id, found_kind, CAST(NULL AS Nullable(DateTime64(3))) AS seen_at,
           toUInt8(1) AS is_current, content_form, content
    FROM current_short_values

    UNION ALL

    SELECT tenant_id, insight_source_id, issue_id, field_id, found_kind, CAST(seen_at AS Nullable(DateTime64(3))) AS seen_at,
           toUInt8(0) AS is_current, content_form, ifNull(content, '') AS content
    FROM past_values
),

field_keys AS (
    SELECT
        tenant_id,
        insight_source_id,
        issue_id,
        CAST('field' AS String)                                       AS found_in,
        field_id                                                      AS found_ref,
        found_kind,
        written_key,
        seen_at,
        is_current
    FROM field_values
    ARRAY JOIN arrayDistinct(if(content_form = 'adf_json',
                                {{ jira_adf_issue_keys('content') }},
                                {{ task_issue_keys('content') }})) AS written_key
),

comment_keys AS (
    SELECT
        tenant_id,
        insight_source_id,
        assumeNotNull(issue_id)                                       AS issue_id,
        CAST('comment' AS String)                                     AS found_in,
        comment_id                                                    AS found_ref,
        CAST('comment' AS String)                                     AS found_kind,
        written_key,
        CAST(created_at AS Nullable(DateTime64(3)))                   AS seen_at,
        toUInt8(1)                                                    AS is_current
    FROM {{ ref('jira__task_comments') }} FINAL
    ARRAY JOIN arrayDistinct({{ jira_adf_issue_keys('COALESCE(body, \'\')') }}) AS written_key
    WHERE ifNull(is_deleted, 0) = 0
      AND issue_id IS NOT NULL
),

written_keys AS (
    SELECT tenant_id, insight_source_id, issue_id, found_in, found_ref, found_kind, written_key, seen_at, is_current
    FROM field_keys

    UNION ALL

    SELECT tenant_id, insight_source_id, issue_id, found_in, found_ref, found_kind, written_key, seen_at, is_current
    FROM comment_keys
),

project_keys AS (
    SELECT DISTINCT tenant_id, insight_source_id, project_key
    FROM {{ ref('jira__task_projects') }}
    WHERE project_key != ''
),

-- A key-shaped token is a reference only when it names an issue, or at least a
-- project the connector syncs; `UTF-8` is neither.
resolved_keys AS (
    SELECT
        w.tenant_id                                                   AS tenant_id,
        w.insight_source_id                                           AS insight_source_id,
        w.issue_id                                                    AS issue_id,
        w.found_in                                                    AS found_in,
        w.found_ref                                                   AS found_ref,
        w.found_kind                                                  AS found_kind,
        w.written_key                                                 AS written_key,
        k.issue_id                                                    AS target_id,
        w.seen_at                                                     AS seen_at,
        w.is_current                                                  AS is_current
    FROM written_keys AS w
    LEFT JOIN {{ ref('jira__issue_key_ids') }} AS k FINAL
        ON k.tenant_id = w.tenant_id
       AND k.insight_source_id = w.insight_source_id
       AND k.issue_key = w.written_key
    LEFT JOIN project_keys AS p
        ON p.tenant_id = w.tenant_id
       AND p.insight_source_id = w.insight_source_id
       AND p.project_key = splitByChar('-', w.written_key)[1]
    WHERE k.issue_id IS NOT NULL OR p.project_key IS NOT NULL
),

links AS (
    SELECT
        tenant_id,
        insight_source_id,
        issue_id,
        CAST('link' AS String)                                        AS found_in,
        link_type                                                     AS found_ref,
        CAST('link' AS String)                                        AS found_kind,
        target_readable                                               AS written_key,
        target_id,
        CAST(valid_from AS Nullable(DateTime64(3)))                   AS seen_at,
        toUInt8(valid_to IS NULL)                                     AS is_current
    FROM {{ ref('jira__task_links') }} FINAL
    WHERE target_type = 'issue'
),

references AS (
    SELECT tenant_id, insight_source_id, issue_id, found_in, found_ref, found_kind, written_key, target_id, seen_at, is_current
    FROM resolved_keys

    UNION ALL

    SELECT tenant_id, insight_source_id, issue_id, found_in, found_ref, found_kind, written_key, target_id, seen_at, is_current
    FROM links
),

-- Renamed here so the aggregates below can take the plain names back.
other_issues AS (
    SELECT
        tenant_id,
        insight_source_id,
        issue_id,
        found_in,
        found_ref,
        found_kind,
        ifNull(target_id, written_key)                                AS target_ident,
        target_id                                                     AS candidate_id,
        written_key                                                   AS candidate_key,
        seen_at                                                       AS candidate_seen_at,
        is_current                                                    AS candidate_current
    FROM references
    WHERE ifNull(target_id, '') != issue_id
),

-- One row per target, whichever of its keys the text wrote: a moved issue's old
-- and new key are the same reference.
per_target AS (
    SELECT
        tenant_id,
        insight_source_id,
        issue_id,
        found_in,
        found_ref,
        found_kind,
        target_ident,
        any(candidate_id)                                             AS target_id,
        argMax(candidate_key, ifNull(candidate_seen_at, toDateTime64('2100-01-01', 3))) AS written_key,
        min(candidate_seen_at)                                        AS seen_at,
        max(candidate_current)                                        AS is_current
    FROM other_issues
    GROUP BY tenant_id, insight_source_id, issue_id, found_in, found_ref, found_kind, target_ident
)

SELECT
    concat(ifNull(r.tenant_id, ''), '-', r.insight_source_id, '-jira-', r.issue_id, '-', r.found_in, '-', r.found_ref, '-',
           r.target_ident)                                            AS unique_key,
    ifNull(r.tenant_id, '')                                           AS tenant_id,
    r.insight_source_id                                               AS insight_source_id,
    CAST('jira' AS String)                                            AS data_source,
    r.issue_id                                                        AS issue_id,
    COALESCE(own.current_key, '')                                     AS id_readable,
    r.target_id                                                       AS target_id,
    COALESCE(target.current_key, r.written_key)                       AS target_key,
    r.found_in                                                        AS found_in,
    r.found_ref                                                       AS found_ref,
    r.found_kind                                                      AS found_kind,
    COALESCE(r.seen_at, i.created_at, toDateTime64(0, 3))             AS first_seen_at,
    r.is_current                                                      AS is_current,
    toUnixTimestamp64Milli(now64(3))                                  AS _version
FROM per_target AS r
LEFT JOIN current_keys AS own
    ON own.tenant_id = r.tenant_id
   AND own.insight_source_id = r.insight_source_id
   AND own.issue_id = r.issue_id
LEFT JOIN current_keys AS target
    ON target.tenant_id = r.tenant_id
   AND target.insight_source_id = r.insight_source_id
   AND target.issue_id = r.target_id
LEFT JOIN issues AS i
    ON i.tenant_id = r.tenant_id
   AND i.insight_source_id = r.insight_source_id
   AND i.issue_id = r.issue_id
