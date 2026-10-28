{{ config(
    severity='warn',
    tags=['connector_quality', 'jira'],
    store_failures=true,
    meta={
        'title': 'Jira bronze rows carry the identity the stream schemas require',
        'domain': 'task-tracking',
        'category': 'consistency',
        'tier': 'warn',
        'remediation': 'Every Jira stream declares tenant_id and source_id required, and jira_issue, jira_issue_keys and jira_issue_census also jira_id; the connector stamps them on every record. The Jira models read these columns through CAST(... AS String), so one NULL stops the Jira transform with Code 349 (Cannot convert NULL value to non-Nullable type), which names neither table nor column; this test does. Run it on its own (dbt test --select assert_jira_identity_columns_are_set) to find the row, find how it was written (a manual insert or a restore from an older dump), then delete or re-sync it.'
    }
) }}

-- Where the NULL is that stops the Jira transform: the models read the identity
-- columns through `CAST(... AS String)`, whose error names no table or column.

{%- set stamped = [
    'jira_board_configuration', 'jira_boards', 'jira_comments', 'jira_fields', 'jira_issue',
    'jira_issue_census', 'jira_issue_history', 'jira_issue_keys', 'jira_issuetypes', 'jira_priorities',
    'jira_project_visibility', 'jira_projects', 'jira_resolutions', 'jira_sprints', 'jira_statuses',
    'jira_user', 'jira_worklog_deleted', 'jira_worklogs'
] %}
{%- set issue_level = ['jira_issue', 'jira_issue_keys', 'jira_issue_census'] %}

{%- for table in stamped %}
{%- set columns = ['tenant_id', 'source_id'] + (['jira_id'] if table in issue_level else []) %}
{%- for column in columns %}

SELECT
    '{{ table }}'                                                     AS bronze_table,
    '{{ column }}'                                                    AS missing_column,
    count()                                                           AS rows_without_it
FROM {{ source('bronze_jira', table) }}
WHERE {{ column }} IS NULL
HAVING rows_without_it > 0
{%- if not loop.last %}

UNION ALL
{%- endif %}
{%- endfor %}
{%- if not loop.last %}

UNION ALL
{%- endif %}
{%- endfor %}
