{{ config(
    tags=['connector_quality', 'youtrack'],
    meta={
        'title': 'Field history events of an issue field have distinct orders',
        'domain': 'task-tracking',
        'category': 'integrity',
        'tier': 'error',
        'remediation': 'Two events of one issue field share an event_order, so their sequence is ambiguous. Check the ranks youtrack__activity_order assigns to same-instant events.'
    }
) }}

SELECT tenant_id, insight_source_id, issue_id, field_id, event_order
FROM {{ ref('youtrack__task_field_history') }} FINAL
GROUP BY tenant_id, insight_source_id, issue_id, field_id, event_order
HAVING count() > 1
