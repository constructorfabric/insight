-- Drop the duplicated `fields` column from bronze_jira.jira_issue.
--
-- The issue payload was stored twice: once as the raw `fields` object the API
-- returns, and once as `custom_fields_json`, the same object re-serialized by
-- the connector. The key sets are identical and every value difference is
-- nested-key ordering, so no information is lost by keeping one — and
-- `custom_fields_json` is the one with readers and with a deterministic key
-- order.
--
-- ORDER MATTERS, and getting it wrong is silent. The connector must stop
-- emitting the field FIRST (descriptor 5.3.0, a RemoveFields on the jira_issue
-- stream plus the schema property) and one sync must complete on the new
-- manifest. Dropping the column while the destination still receives it in the
-- catalog simply recreates it on the next write.
--
-- `DROP COLUMN IF EXISTS`, so re-running is a no-op and a cluster that never
-- had the column is unaffected. The table itself is not guarded: ClickHouse has
-- no `ALTER TABLE IF EXISTS`, and the migrations run after the bronze
-- placeholders exist.
--
-- See src/ingestion/connectors/task-tracking/jira/specs/FIELD-HISTORY-IN-DBT.md
-- §12.

ALTER TABLE bronze_jira.jira_issue
    DROP COLUMN IF EXISTS fields;
