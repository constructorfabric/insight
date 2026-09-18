-- Give the three issue-scoped Jira substreams a column for the issue's
-- immutable id. Connector 6.1.0 fills it on new rows; the rows already stored
-- are filled by `heal_jira_substream_issue_id` in apply-ch-migrations.sh, which
-- runs after this file and only while rows still lack the value.
--
-- Idempotent: IF NOT EXISTS, and `jira_id` is not part of the sorting key.

ALTER TABLE bronze_jira.jira_issue_history
    ADD COLUMN IF NOT EXISTS jira_id Nullable(String) AFTER id_readable;

ALTER TABLE bronze_jira.jira_comments
    ADD COLUMN IF NOT EXISTS jira_id Nullable(String) AFTER id_readable;

ALTER TABLE bronze_jira.jira_worklogs
    ADD COLUMN IF NOT EXISTS jira_id Nullable(String) AFTER id_readable;
