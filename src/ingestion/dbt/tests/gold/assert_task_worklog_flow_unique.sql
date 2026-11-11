SELECT
    tenant_id,
    entity_id,
    account_source_type,
    account_source_id,
    account_id,
    metric_date,
    insight_source_id,
    issue_id,
    count() AS row_count
FROM {{ ref('task_worklog_flow') }}
GROUP BY tenant_id, entity_id, account_source_type, account_source_id, account_id,
         metric_date, insight_source_id, issue_id
HAVING count() > 1
