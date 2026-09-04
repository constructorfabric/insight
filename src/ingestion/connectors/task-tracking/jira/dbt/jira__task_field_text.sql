-- depends_on: {{ ref('jira__bronze_promoted') }}
-- depends_on: {{ ref('jira__task_field_kind') }}
-- depends_on: {{ ref('jira__changelog_items') }}
{{ config(
    materialized='table',
    alias='jira__task_field_text',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by=['text_id'],
    query_settings={
        'max_bytes_before_external_group_by': 2000000000,
        'max_bytes_before_external_sort': 2000000000,
    },
    tags=['staging', 'jira']
) }}

-- Bodies of long-text fields, addressed by a hash of their content. The journal
-- stores the hash in `value_ids` and a short prefix in `value_displays`; the
-- full body lives here. See
-- `connectors/task-tracking/jira/specs/FIELD-HISTORY-IN-DBT.md` §8.
--
-- WHY a side table rather than the body inline. ClickHouse is column-oriented,
-- so a query that reads `value_displays` for a status field also reads whatever
-- else that column holds. Descriptions run to tens of kilobytes, so keeping them
-- in the journal's array columns would drag them through every read of any
-- field. Content-addressing also collapses repeated bodies — a body that is
-- re-saved unchanged, or shared across events, stores once.
--
-- INVARIANT: `text_id` is a pure function of `content`, so the same body always
-- resolves to the same row and ReplacingMergeTree collapses duplicates without
-- a version race.
--
-- The two sides of the pipeline carry long text in DIFFERENT representations and
-- both are kept, distinguished by `content_form`:
--
--   * `adf_json`      — the issue JSON holds an Atlassian Document Format
--                       document, the authoritative current body;
--   * `rendered_text` — the changelog holds Jira's plain-text rendering of the
--                       body at that point in time. It is not truncated (bodies
--                       well past 32 KiB arrive whole), but it is a rendering:
--                       formatting, links and embedded media are flattened.
--
-- Because the two forms are not comparable, `long_text` is exempt from the
-- round-trip invariant (§14) — that exemption is a consequence of this
-- asymmetry, not an oversight.

WITH long_text_fields AS (
    SELECT insight_source_id, field_id
    FROM {{ ref('jira__task_field_kind') }}
    WHERE field_kind = 'long_text'
),

-- Current bodies, from the issue JSON.
issue_winner AS (
    SELECT unique_key, argMax(_airbyte_raw_id, _airbyte_extracted_at) AS raw_id
    FROM {{ source('bronze_jira', 'jira_issue') }}
    WHERE unique_key IS NOT NULL
    GROUP BY unique_key
),

from_snapshot AS (
    SELECT
        kv.2                                              AS content,
        CAST('adf_json' AS String)                        AS content_form
    FROM {{ source('bronze_jira', 'jira_issue') }} AS i
    INNER JOIN issue_winner AS w ON i._airbyte_raw_id = w.raw_id
    ARRAY JOIN JSONExtractKeysAndValuesRaw(COALESCE(i.custom_fields_json, '{}')) AS kv
    INNER JOIN long_text_fields AS f
        ON f.insight_source_id = COALESCE(i.source_id, '')
       AND f.field_id = kv.1
    WHERE kv.2 NOT IN ('', 'null', '""', '{}')
),

-- Historical bodies, from both sides of every changelog item.
from_changelog AS (
    SELECT
        content,
        CAST('rendered_text' AS String)                   AS content_form
    FROM (
        SELECT ci.value_from_string AS content
        FROM {{ ref('jira__changelog_items') }} AS ci
        INNER JOIN long_text_fields AS f
            ON f.insight_source_id = ci.insight_source_id
           AND f.field_id = ci.field_id
        UNION ALL
        SELECT ci.value_to_string AS content
        FROM {{ ref('jira__changelog_items') }} AS ci
        INNER JOIN long_text_fields AS f
            ON f.insight_source_id = ci.insight_source_id
           AND f.field_id = ci.field_id
    )
    WHERE content IS NOT NULL AND content != ''
),

all_text AS (
    -- Non-Nullable on purpose: the changelog columns are Nullable, and a
    -- Nullable content makes `text_id` Nullable too — which cannot be a sorting
    -- key. Both arms already exclude NULL, so the cast asserts what is true.
    SELECT CAST(assumeNotNull(content) AS String) AS content, content_form
    FROM from_snapshot
    UNION ALL
    SELECT CAST(assumeNotNull(content) AS String) AS content, content_form
    FROM from_changelog
)

-- `text_id` is computed in a subquery, not as a GROUP BY alias: ClickHouse
-- resolves the alias inside the GROUP BY and rejects the aggregate it wraps.
--
-- No `content_length` column: `length(content)` gives it, and an output alias
-- named `content` shadows the input column, so any expression wrapping it here
-- parses as nesting one aggregate inside another.
SELECT
    text_id,
    any(content_form)                                     AS content_form,
    any(content)                                          AS content,
    toUnixTimestamp64Milli(now64(3))                      AS _version
FROM (
    SELECT
        {{ jira_text_id('content') }} AS text_id,
        content_form,
        content
    FROM all_text
)
GROUP BY text_id
