{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'Kind subsets partition tasks closed exactly',
        'domain': 'task',
        'category': 'reconciliation',
        'tier': 'error',
        'remediation': 'bugs_fixed + closed_task + closed_unknown_type != tasks_closed for an entity-date means the closed-issue evidence fanned out, a measure branch double-emitted, or a kind fell through all three branches. Check the issue_state dedup, the issue_kind mapping and the ARRAY JOIN measure branches in task_metric_evidence.'
    }
) }}
-- The four measures are minted by one ARRAY JOIN per closed issue in
-- task_metric_evidence: tasks_closed always, then exactly one of bugs_fixed /
-- closed_task / closed_unknown_type by issue_kind. So per
-- (tenant, source, entity, date) the three subsets partition tasks_closed by
-- construction; a violation indicates fan-out, double-emission or an
-- unbranched kind.
-- Dimensions are aggregated over, not grouped by: the measures carry
-- (type, source) dimension tuples while the invariant is per entity-date.
SELECT
    tenant_id,
    source_key,
    entity_type,
    entity_id,
    metric_date,
    sumIf(ifNull(value, 0), measure_key = 'bugs_fixed') AS bugs_fixed,
    sumIf(ifNull(value, 0), measure_key = 'closed_task') AS closed_task,
    sumIf(ifNull(value, 0), measure_key = 'closed_unknown_type') AS closed_unknown_type,
    sumIf(ifNull(value, 0), measure_key = 'tasks_closed') AS tasks_closed
FROM {{ ref('task_metric_observations') }}
WHERE measure_key IN ('bugs_fixed', 'closed_task', 'closed_unknown_type', 'tasks_closed')
GROUP BY tenant_id, source_key, entity_type, entity_id, metric_date
HAVING bugs_fixed + closed_task + closed_unknown_type != tasks_closed
