-- depends_on: {{ ref('jira__bronze_promoted') }}
{{ config(
    materialized='view',
    alias='jira__task_statuses',
    schema='staging',
    tags=['jira', 'silver:class_task_statuses']
) }}

-- Per-source status dimension; unioned into `silver.class_task_statuses` via `union_by_tag`.
-- Maps every Jira status id to the source-neutral lifecycle `status_category`
-- (new / in_progress / done / undefined), so Gold detects "done" without matching
-- localized status display names. See docs task-tracking silver DESIGN
-- (`cpt-insightspec-dbtable-tt-silver-statuses`) and issue #1541.
--
-- View, not table: bronze `jira_statuses` is MergeTree (full_refresh + overwrite),
-- so the current state of bronze is the current state of staging. FINAL not needed.
--
-- Jira statusCategory is stable and locale-independent:
--   key='new'          (id 2) -> new
--   key='indeterminate'(id 4) -> in_progress
--   key='done'         (id 3) -> done
--   key='undefined'    (id 1) -> undefined
-- `category_key` is the primary signal (added to the `jira_statuses` stream);
-- the numeric `category_id` is the fallback when the key is absent.
--
-- `status_id` is normalised to an integer-string (stripping any `.0` from
-- Airbyte numeric coercion) so it joins `class_task_field_history.value_ids[1]`,
-- which carries the Jira status id as a plain string (e.g. '10007').

SELECT
    s.unique_key                                            AS unique_key,
    s.source_id                                             AS insight_source_id,
    CAST('jira' AS String)                                  AS data_source,
    replaceRegexpOne(toString(s.status_id), '\.0+$', '')    AS status_id,
    s.name                                                  AS status_name,
    toInt32OrNull(toString(s.category_id))                  AS category_id,
    nullIf(toString(s.category_key), '')                    AS category_key,
    multiIf(
        lower(toString(s.category_key)) = 'done',          'done',
        lower(toString(s.category_key)) = 'new',           'new',
        lower(toString(s.category_key)) = 'indeterminate', 'in_progress',
        lower(toString(s.category_key)) = 'undefined',     'undefined',
        toInt32OrNull(toString(s.category_id)) = 3,         'done',
        toInt32OrNull(toString(s.category_id)) = 2,         'new',
        toInt32OrNull(toString(s.category_id)) = 4,         'in_progress',
        'undefined'
    )                                                       AS status_category,
    toDateTime64(s._airbyte_extracted_at, 3)                AS collected_at,
    toUnixTimestamp64Milli(s._airbyte_extracted_at)         AS _version
FROM {{ source('bronze_jira', 'jira_statuses') }} s
-- `jira_statuses` bronze = MergeTree (full_refresh + overwrite), FINAL not supported.
