-- depends_on: {{ ref('jira__bronze_promoted') }}
{{ config(
    materialized='view',
    alias='jira__task_field_unclassified',
    schema='staging',
    tags=['staging', 'jira']
) }}

-- Every field the changelog names that the field catalogue does not contain, so
-- the exclusion is queryable rather than implicit. See
-- `connectors/task-tracking/jira/specs/FIELD-HISTORY-IN-DBT.md` §3.2.
--
-- These fields cannot be classified even in principle: without a catalogue row
-- there is no `schema_type` to read, so no separator, no id side and no
-- cardinality. Their history is therefore not reconstructed — the journal
-- carries one best-effort `unclassified_field` row per (issue, field) with the
-- last value as it arrived.
--
-- A view: it is a small aggregate over staging, and the current state of the
-- changelog is the current state of the answer.
--
-- What to do with a row here depends on `newest_event`, which is why the column
-- exists. Bronze is append-only with dedup per field, so the catalogue never
-- forgets a field it has seen once:
--
--   * events older than the catalogue's own first sync — the field was deleted
--     before the connector ever looked. Legitimately unclassifiable, and
--     nothing can be done;
--   * events newer than that — the field exists (or existed recently) and its
--     metadata has not arrived. That is a collection gap, not a dead field, and
--     `assert_jira_unclassified_fields_are_old` fails on it.
--
-- The first sync comes from `jira__catalogue_first_seen`, not from bronze: the
-- catalogue table forgets its older extractions as its parts merge.

SELECT
    ci.insight_source_id                                  AS insight_source_id,
    CAST('jira' AS String)                                AS data_source,
    ci.field_id                                           AS field_id,
    -- The changelog item's own display name, which is all the naming there is.
    any(ci.field_name)                                    AS field_name,
    count()                                               AS changelog_items,
    uniqExact(ci.id_readable)                             AS issues_affected,
    min(ci.created_at)                                    AS oldest_event,
    max(ci.created_at)                                    AS newest_event,
    any(c.catalogue_first_sync)                           AS catalogue_first_sync,
    max(ci.created_at) > any(c.catalogue_first_sync)      AS metadata_is_missing
FROM {{ ref('jira__changelog_items') }} AS ci
LEFT ANTI JOIN {{ ref('jira__task_field_kind') }} AS k
    ON k.insight_source_id = ci.insight_source_id
   AND k.field_id = ci.field_id
LEFT JOIN {{ ref('jira__catalogue_first_seen') }} AS c
    ON c.insight_source_id = ci.insight_source_id
GROUP BY ci.insight_source_id, ci.field_id
