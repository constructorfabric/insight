{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['tenant_id', 'assignee_email', 'closed_date'],
    partition_by='toYYYYMM(closed_date)',
    schema=var('gold_database'),
    tags=['gold'],
    query_settings=metric_serving_query_settings()
) }}

-- Estimation against reality, per person and closing day: the day's average
-- estimate as a percentage of the day's average time spent.
--
-- INVARIANT: the ratio is taken between two daily averages, never between per
-- issue pairs — a person's day is the unit of estimation, so one long-running
-- issue cannot dominate the figure the way a per-issue ratio lets it.

WITH
estimated AS (
    SELECT
        tenant_id                                                            AS tenant_id,
        assignee_email                                                       AS assignee_email,
        closed_date                                                          AS closed_date,
        100 * avgIf(estimate_seconds, is_closed = 1
                                      AND ifNull(estimate_seconds, 0) > 0
                                      AND spent_seconds IS NOT NULL)
            / nullIf(avgIf(spent_seconds, is_closed = 1
                                          AND ifNull(estimate_seconds, 0) > 0
                                          AND spent_seconds IS NOT NULL), 0) AS estimation_pct
    FROM {{ ref('task_closed_issues') }}
    GROUP BY tenant_id, assignee_email, closed_date
)

SELECT
    tenant_id                                                                AS tenant_id,
    assignee_email                                                           AS assignee_email,
    closed_date                                                              AS closed_date,
    estimation_pct                                                           AS estimation_pct,
    abs(100 - estimation_pct)                                                AS estimation_error_pct
FROM estimated
WHERE estimation_pct IS NOT NULL
