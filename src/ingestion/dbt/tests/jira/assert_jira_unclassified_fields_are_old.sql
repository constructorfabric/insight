-- A field excluded for catalogue absence must be an OLD field.
--
-- FIELD-HISTORY-IN-DBT.md §3.2. Absence from `bronze_jira.jira_fields` has two
-- causes that look identical from the outside, and only one of them is
-- acceptable:
--
--   * the field was deleted from the instance BEFORE the connector's first field
--     sync. Bronze is append-only with dedup per field, so the catalogue never
--     forgets a field it has seen once — such a field can only have events older
--     than that first sync. Nothing downstream can classify it, and nothing
--     needs to;
--   * the field was created since the last field sync, so its events arrive
--     before its metadata. Indistinguishable by absence alone, and silently
--     dropping it is exactly the defect this design removes — a populated field
--     with no history at all.
--
-- The event timestamp separates them. An event newer than the catalogue's own
-- first sync means the field is not ancient, so its metadata is missing rather
-- than gone: re-sync the field stream, or add an override row.
--
-- This is deliberately not a `throwIf` in a model. The condition is about
-- collection, not about a shape the model cannot handle, and the journal still
-- carries the best-effort rows meanwhile.

SELECT
    insight_source_id,
    field_id,
    field_name,
    changelog_items,
    issues_affected,
    oldest_event,
    newest_event,
    catalogue_first_sync
FROM {{ ref('jira__task_field_unclassified') }}
WHERE metadata_is_missing
LIMIT 100
