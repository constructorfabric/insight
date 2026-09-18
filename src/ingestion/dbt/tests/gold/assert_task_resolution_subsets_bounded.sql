{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'Resolution-class closures never exceed tasks closed',
        'domain': 'task',
        'category': 'physical_bound',
        'tier': 'error',
        'remediation': 'closed_fixed + closed_duplicate + closed_wontfix > tasks_closed for an entity-date means the closed-issue evidence fanned out or a measure branch double-emitted. Check the issue_state dedup and the ARRAY JOIN measure branches in task_metric_evidence.'
    }
) }}
-- The measures are minted by one ARRAY JOIN per closed issue in
-- task_metric_evidence: tasks_closed always, then closed_fixed /
-- closed_duplicate / closed_wontfix iff the resolution class matches
-- (class = unknown mints none). So per (tenant, source, entity, date) the
-- subsets sum to at most tasks_closed by construction; a violation indicates
-- fan-out or double-emission. Bounded, not exact: the unknown class has no
-- measure, so the resolution axis does not close. The kind axis does, and
-- `assert_task_kind_subsets_partition_closed` asserts it as an equality.
-- Dimensions are aggregated over, not grouped by: the measures carry
-- (type, source, resolution) dimension tuples while the invariant is per
-- entity-date.
SELECT
    tenant_id,
    source_key,
    entity_type,
    entity_id,
    metric_date,
    sumIf(ifNull(value, 0), measure_key = 'closed_fixed') AS closed_fixed,
    sumIf(ifNull(value, 0), measure_key = 'closed_duplicate') AS closed_duplicate,
    sumIf(ifNull(value, 0), measure_key = 'closed_wontfix') AS closed_wontfix,
    sumIf(ifNull(value, 0), measure_key = 'tasks_closed') AS tasks_closed
FROM {{ ref('task_metric_observations') }}
WHERE measure_key IN ('closed_fixed', 'closed_duplicate', 'closed_wontfix', 'tasks_closed')
GROUP BY tenant_id, source_key, entity_type, entity_id, metric_date
HAVING closed_fixed + closed_duplicate + closed_wontfix > tasks_closed
