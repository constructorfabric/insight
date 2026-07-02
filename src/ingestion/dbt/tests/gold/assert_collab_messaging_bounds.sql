{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'collab_person_counter_daily messaging counters within sane bounds',
        'domain': 'gold',
        'category': 'physical_bound',
        'tier': 'error',
        'remediation': 'A collaboration messaging counter is negative. messages_sent and channel_posts are period totals and must be non-negative by construction. Inspect the insight.collab_person_counter_daily view (#1527) — a row here usually means a UNION-branch sign error or a join fanout.'
    }
) }}
-- Gold-layer check. `insight.collab_person_counter_daily` is a database view
-- (not a dbt model); a singular test reads it via the registered `gold` source.
-- The Messaging counters are honest-NULL period totals, so NULL is legal but a
-- negative value never is. A violation is one or more rows. Grain: one row per
-- (person_id, metric_date).
SELECT
    person_id,
    metric_date,
    messages_sent,
    channel_posts
FROM {{ source('gold', 'collab_person_counter_daily') }}
WHERE (messages_sent IS NOT NULL AND messages_sent < 0)
   OR (channel_posts IS NOT NULL AND channel_posts < 0)
