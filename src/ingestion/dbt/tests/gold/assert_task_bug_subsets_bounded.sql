{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'Bug and non-bug closures never exceed tasks closed',
        'domain': 'task',
        'category': 'physical_bound',
        'tier': 'error',
        'remediation': 'A kind or resolution subset sum exceeding tasks_closed for an entity-date means the closed-issue evidence fanned out or a measure branch double-emitted. Check the issue_state dedup and the ARRAY JOIN measure branches in task_metric_evidence.'
    }
) }}
-- The measures are minted by one ARRAY JOIN per closed issue in
-- task_metric_evidence: tasks_closed always, bugs_fixed iff kind = bug,
-- closed_non_bug iff kind = other (kind = unknown mints neither), and
-- closed_fixed / closed_duplicate / closed_wontfix iff the resolution class
-- matches (class = unknown mints none). So per (tenant, source, entity, date)
-- each partition's subsets sum to at most tasks_closed by construction; a
-- violation indicates fan-out or double-emission.
-- Dimensions are aggregated over, not grouped by: the measures carry
-- (type, source) dimension tuples while the invariant is per entity-date.
SELECT
    tenant_id,
    source_key,
    entity_type,
    entity_id,
    metric_date,
    sumIf(ifNull(value, 0), measure_key = 'bugs_fixed') AS bugs_fixed,
    sumIf(ifNull(value, 0), measure_key = 'closed_non_bug') AS closed_non_bug,
    sumIf(ifNull(value, 0), measure_key = 'closed_fixed') AS closed_fixed,
    sumIf(ifNull(value, 0), measure_key = 'closed_duplicate') AS closed_duplicate,
    sumIf(ifNull(value, 0), measure_key = 'closed_wontfix') AS closed_wontfix,
    sumIf(ifNull(value, 0), measure_key = 'tasks_closed') AS tasks_closed
FROM {{ ref('task_metric_observations') }}
WHERE measure_key IN ('bugs_fixed', 'closed_non_bug', 'closed_fixed', 'closed_duplicate', 'closed_wontfix', 'tasks_closed')
GROUP BY tenant_id, source_key, entity_type, entity_id, metric_date
HAVING bugs_fixed + closed_non_bug > tasks_closed
    OR closed_fixed + closed_duplicate + closed_wontfix > tasks_closed
