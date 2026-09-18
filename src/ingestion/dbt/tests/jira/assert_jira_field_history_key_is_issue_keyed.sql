{{ config(tags=['jira', 'staging']) }}

-- Every journal row's `unique_key` is the one formula `jira_history_key` states,
-- with the issue named by its immutable id. A row keyed any other way — by the
-- readable key, or by a hand-built concat that drifted from the macro —
-- duplicates the issue's history the first time its key changes (#2741).
-- Recomputed from the row's own columns, so a producer cannot disagree with
-- the contract without this failing. An empty `issue_id` fails it too.

SELECT unique_key, issue_id, id_readable, field_id, event_id
FROM {{ ref('jira__field_history_derived') }} FINAL
WHERE issue_id = ''
   OR unique_key != {{ jira_history_key('insight_source_id', 'issue_id', 'field_id', 'event_id') }}
LIMIT 100
