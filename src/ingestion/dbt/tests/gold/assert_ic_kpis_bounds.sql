{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'ic_kpis metric values within sane bounds',
        'domain': 'gold',
        'category': 'physical_bound',
        'tier': 'error',
        'remediation': 'A Gold KPI is out of range: focus_time_pct must be within [0, 100] and counts must be non-negative. Inspect the insight.ic_kpis view definition — a row here usually means a join fanout or a broken aggregate.'
    }
) }}
-- Gold-layer check. `insight.ic_kpis` is a database view (not a dbt model); a
-- singular test reads it the same way via the registered `gold` source. These
-- bounds must always hold by construction, so any returned row means the view
-- math is wrong. A violation is one or more rows.
SELECT
    person_id,
    metric_date,
    focus_time_pct,
    tasks_closed,
    bugs_fixed,
    ai_sessions
FROM {{ source('gold', 'ic_kpis') }}
WHERE (focus_time_pct IS NOT NULL AND (focus_time_pct < 0 OR focus_time_pct > 100))
   OR (tasks_closed   IS NOT NULL AND tasks_closed   < 0)
   OR (bugs_fixed     IS NOT NULL AND bugs_fixed     < 0)
   OR (ai_sessions    IS NOT NULL AND ai_sessions    < 0)
