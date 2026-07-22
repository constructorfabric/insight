{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['insight_source_id', 'issue_id', 'interval_start'],
    schema='insight',
    alias='task_status_spans',
    tags=['gold'],
    query_settings={
        'max_memory_usage': 1610612736,
        'max_threads': 4,
        'max_bytes_before_external_group_by': 805306368,
        'max_bytes_before_external_sort': 805306368
    }
) }}

-- Per-issue status spans: each status event paired with the next. The last
-- span ends per CURRENT state — at the close for currently-done issues, live
-- to build time otherwise. Keying the tail on the close time alone would end
-- a reopened issue's current span before it starts; the row filter below
-- would then drop it, hiding the reopen from transition detection and
-- freezing its in-progress accrual.
--
-- Materialized once per build (ClickHouse re-inlines every WITH reference);
-- the class read keeps FINAL (RMT parts are not duplicate-immune).

WITH
status_events AS (
    SELECT
        insight_source_id,
        issue_id,
        arraySort(x -> x.1, groupArray((event_at, value_ids[1]))) AS evs
    FROM {{ ref('class_task_field_history') }} FINAL
    WHERE field_id = 'status' AND delta_action = 'set'
    GROUP BY insight_source_id, issue_id
)
SELECT
    iv.insight_source_id                                                     AS insight_source_id,
    iv.issue_id                                                              AS issue_id,
    iv.interval_start                                                        AS interval_start,
    iv.interval_end                                                          AS interval_end,
    st.status_category                                                       AS status_category,
    iv.duration_seconds                                                      AS duration_seconds
FROM (
    SELECT
        e.insight_source_id                                                  AS insight_source_id,
        e.issue_id                                                           AS issue_id,
        arrayJoin(arrayMap(
            i -> (
                (e.evs[i]).1,
                if(i = length(e.evs),
                   if(s.status_category = 'done',
                      ifNull(s.final_close_at, (e.evs[i]).1),
                      now()),
                   (e.evs[i + 1]).1),
                (e.evs[i]).2
            ),
            range(1, length(e.evs) + 1)
        ))                                                                   AS row,
        row.1                                                                AS interval_start,
        row.2                                                                AS interval_end,
        row.3                                                                AS status_id,
        toFloat64(greatest(toInt64(0), dateDiff('second', row.1, row.2)))    AS duration_seconds,
        s.created_at                                                         AS issue_created_at
    FROM status_events AS e
    INNER JOIN {{ ref('task_issue_state') }} AS s
        ON s.insight_source_id = e.insight_source_id AND s.issue_id = e.issue_id
) AS iv
LEFT JOIN {{ ref('class_task_statuses') }} AS st FINAL
    ON st.insight_source_id = iv.insight_source_id AND st.status_id = iv.status_id
WHERE iv.interval_start >= ifNull(iv.issue_created_at, toDateTime('1970-01-02'))
  AND iv.interval_end >= iv.interval_start
  AND iv.interval_end <= now() + INTERVAL 1 DAY
