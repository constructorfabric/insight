{{ config(materialized='table', schema='staging', engine=insight_engine('ReplacingMergeTree', '_version'), order_by=['unique_key'], tags=['youtrack', 'staging']) }}

WITH events AS (
    SELECT a.*, r.entry_rank,
        {{ task_event_order('a.event_at', '1', 'r.entry_rank') }} AS event_order
    FROM {{ ref('youtrack__activities') }} AS a
    INNER JOIN {{ ref('youtrack__activity_order') }} AS r USING (insight_source_id, issue_id, event_id)
), grouped AS (
    SELECT insight_source_id, issue_id, field_id,
        arraySort(e -> e.1, groupArray((event_order, event_id, before_pairs, after_pairs, field_cardinality))) AS events
    FROM events GROUP BY insight_source_id, issue_id, field_id
), latest AS (
    SELECT insight_source_id, issue_id, field_id, argMax(pairs, observed_at) AS pairs
    FROM {{ ref('youtrack__issue_field_snapshot') }}
    GROUP BY insight_source_id, issue_id, field_id
), candidates AS (
    SELECT g.*, s.pairs AS snapshot_pairs,
        arrayDistinct(arrayMap(p -> p.1, arrayFlatten(arrayMap(e -> arrayConcat(e.3, e.4), events)))) AS touched_ids
    FROM grouped AS g
    LEFT JOIN latest AS s USING (insight_source_id, issue_id, field_id)
), initial AS (
    -- A single-valued field starts from what its first change replaced. The
    -- id-merge below is for sets: applied to a single value it keeps the old
    -- one whenever the replaced text differs from it by a byte (trailing
    -- whitespace), and the field ends up holding two values.
    SELECT *, if(events[1].5 = 'single', events[1].3, arrayConcat(
        arrayFilter(p -> NOT has(touched_ids, p.1), snapshot_pairs),
        arrayFlatten(arrayMap(id -> arrayFilter(p -> p.1 = id,
            arrayFirst(e -> has(arrayMap(p -> p.1, arrayConcat(e.3, e.4)), id), events).3), touched_ids))
    )) AS initial_pairs
    FROM candidates
), replay AS (
    SELECT *, arrayFold((states, e) -> arrayPushBack(states,
        if(e.5 = 'single', e.4, arraySort(p -> p.1, arrayConcat(
            arrayFilter(p -> NOT has(arrayMap(v -> v.1, arrayConcat(e.3, e.4)), p.1), states[-1]), e.4)))),
        events, [initial_pairs]) AS states
    FROM initial
)
SELECT concat(insight_source_id, '-youtrack-', issue_id, '-', field_id, '-', event_id) AS unique_key,
    insight_source_id, issue_id, field_id, event_id, pairs,
    toUInt64(toUnixTimestamp64Milli(now64(3))) AS _version
FROM replay ARRAY JOIN
    arrayMap(n -> if(n = 0, concat('initial:', issue_id), events[n].2), range(0, length(events)+1)) AS event_id,
    arrayMap(n -> states[n+1], range(0, length(events)+1)) AS pairs
