{{ config(
    materialized='table',
    schema='staging',
    tags=['youtrack', 'staging', 'silver:class_task_field_history'],
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    query_settings={
        'max_bytes_before_external_group_by': 2000000000,
        'max_bytes_before_external_sort': 2000000000,
    },
) }}

WITH changes AS (
    SELECT a.tenant_id AS tenant_id, a.insight_source_id AS insight_source_id, a.issue_id AS issue_id,
        i.id_readable AS id_readable, a.event_id AS event_id, a.event_at AS event_at,
        'changelog' AS event_kind, r.entry_rank AS _seq, a.author_id AS author_id,
        a.field_id AS field_id, a.field_name AS field_name, a.field_cardinality AS field_cardinality,
        fs.pairs AS pairs, a.collected_at AS collected_at, 'set' AS delta_action
    FROM {{ ref('youtrack__activities') }} AS a
    INNER JOIN {{ ref('youtrack__issues') }} AS i USING (tenant_id, insight_source_id, issue_id)
    INNER JOIN {{ ref('youtrack__activity_order') }} AS r USING (tenant_id, insight_source_id, issue_id, event_id)
    INNER JOIN {{ ref('youtrack__field_states') }} AS fs USING (tenant_id, insight_source_id, issue_id, field_id, event_id)
), field_events AS (
    SELECT tenant_id, insight_source_id, issue_id, field_id, event_at FROM {{ ref('youtrack__activities') }}
), snapshots AS (
    SELECT *, lagInFrame(toNullable(observed_at)) OVER (PARTITION BY tenant_id, insight_source_id, issue_id, field_id
        ORDER BY observed_at
        ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS previous_observed_at
    FROM {{ ref('youtrack__issue_field_snapshot') }}
), resolutions AS (
    SELECT tenant_id, insight_source_id, issue_id, observed_at,
        {{ youtrack_timestamp("JSONExtractRaw(payload, 'resolved')") }} AS resolved_at
    FROM {{ ref('youtrack__issue_observations') }} FINAL
), bounded_snapshots AS (
    -- known_at: the field's last event before the snapshot, or its previous snapshot.
    SELECT s.*, r.resolved_at AS resolved_at, nullIf(e.event_at, toDateTime64(0, 3)) AS last_event_at,
        multiIf(last_event_at IS NULL, s.previous_observed_at,
                s.previous_observed_at IS NULL, last_event_at,
                greatest(assumeNotNull(last_event_at), assumeNotNull(s.previous_observed_at))) AS known_at
    FROM snapshots AS s
    ASOF LEFT JOIN field_events AS e
        ON e.tenant_id = s.tenant_id AND e.insight_source_id = s.insight_source_id AND e.issue_id = s.issue_id
        AND e.field_id = s.field_id AND s.observed_at >= e.event_at
    LEFT JOIN resolutions AS r
        ON r.tenant_id = s.tenant_id AND r.insight_source_id = s.insight_source_id AND r.issue_id = s.issue_id
        AND r.observed_at = s.observed_at
), observations AS (
    -- INVARIANT: dated by source facts, never by collection time, so a re-sync cannot move a close time.
    SELECT s.tenant_id, s.insight_source_id, s.issue_id, s.id_readable,
        concat('snapshot_diff:', s.issue_id, ':', toString(toUnixTimestamp64Milli(s.observed_at))) AS event_id,
        multiIf(s.value_is_resolved = 1 AND s.resolved_at IS NOT NULL AND s.resolved_at <= s.observed_at
                    AND (s.known_at IS NULL OR s.resolved_at > s.known_at), assumeNotNull(s.resolved_at),
                s.known_at IS NULL, s.created_at,
                assumeNotNull(s.known_at) + toIntervalMillisecond(1)) AS event_at,
        'snapshot_diff' AS event_kind,
        toUInt32(row_number() OVER (PARTITION BY s.tenant_id, s.insight_source_id, s.issue_id, s.observed_at
            ORDER BY s.field_id)) AS _seq,
        CAST(NULL AS Nullable(String)) AS author_id, s.field_id, s.field_name,
        s.field_cardinality, s.pairs, s.observed_at AS collected_at, 'set' AS delta_action
    FROM bounded_snapshots AS s
), retired AS (
    SELECT tenant_id, insight_source_id, issue_id, id_readable,
        concat('retired:', issue_id, ':', toString(toUnixTimestamp64Milli(observed_at))) AS event_id,
        observed_at AS event_at, 'retired_field' AS event_kind,
        toUInt32(25000 + row_number() OVER (PARTITION BY tenant_id, insight_source_id, issue_id, observed_at
            ORDER BY field_id)) AS _seq,
        CAST(NULL AS Nullable(String)) AS author_id, field_id, field_id AS field_name,
        'unknown' AS field_cardinality, CAST([] AS Array(Tuple(String, String))) AS pairs,
        observed_at AS collected_at, 'set' AS delta_action
    FROM {{ ref('youtrack__retired_fields') }}
), lifecycle AS (
    SELECT l.tenant_id, l.insight_source_id, l.issue_id, i.id_readable, l.event_id, l.event_at, 'lifecycle' AS event_kind,
        toUInt32(50000 + row_number() OVER (PARTITION BY l.tenant_id, l.insight_source_id, l.issue_id, l.event_at
            ORDER BY l.event_id)) AS _seq,
        l.author_id, l.field_id, l.field_id AS field_name, 'multi' AS field_cardinality, l.pairs, l.collected_at, l.delta_action
    FROM {{ ref('youtrack__lifecycle_events') }} AS l
    INNER JOIN {{ ref('youtrack__issues') }} AS i USING (tenant_id, insight_source_id, issue_id)
), feed_starts AS (
    SELECT tenant_id, insight_source_id, min(event_at) AS feed_start_at
    FROM {{ ref('youtrack__activities') }} GROUP BY tenant_id, insight_source_id
), first_changes AS (
    SELECT tenant_id, insight_source_id, issue_id, field_id,
        argMin(field_name, event_at) AS field_name, argMin(field_cardinality, event_at) AS field_cardinality
    FROM {{ ref('youtrack__activities') }} GROUP BY tenant_id, insight_source_id, issue_id, field_id
), initial_states AS (
    SELECT tenant_id, insight_source_id, issue_id, field_id, event_id, pairs
    FROM {{ ref('youtrack__field_states') }} FINAL WHERE startsWith(event_id, 'initial:')
), initials AS (
    -- INVARIANT: only an issue created inside the activity feed has every change since
    -- its creation in hand; earlier, what the first seen change replaced is a guess.
    SELECT s.tenant_id, s.insight_source_id, s.issue_id, i.id_readable, s.event_id, i.created_at AS event_at,
        'synthetic_initial' AS event_kind,
        toUInt32(row_number() OVER (PARTITION BY s.tenant_id, s.insight_source_id, s.issue_id ORDER BY s.field_id)) AS _seq, CAST(NULL AS Nullable(String)) AS author_id,
        s.field_id, c.field_name, c.field_cardinality, s.pairs, i.observed_at AS collected_at, 'set' AS delta_action
    FROM initial_states AS s
    INNER JOIN first_changes AS c
        ON c.tenant_id = s.tenant_id AND c.insight_source_id = s.insight_source_id
        AND c.issue_id = s.issue_id AND c.field_id = s.field_id
    INNER JOIN {{ ref('youtrack__issues') }} AS i
        ON i.tenant_id = s.tenant_id AND i.insight_source_id = s.insight_source_id AND i.issue_id = s.issue_id
    INNER JOIN feed_starts AS f ON f.tenant_id = s.tenant_id AND f.insight_source_id = s.insight_source_id
    WHERE i.created_at >= f.feed_start_at
), origins AS (
    SELECT tenant_id, insight_source_id, issue_id, min(event_at) AS first_event_at
    FROM changes GROUP BY tenant_id, insight_source_id, issue_id
), rows AS (
    SELECT * FROM changes
    UNION ALL
    SELECT * FROM observations
    UNION ALL
    SELECT * FROM lifecycle
    UNION ALL
    SELECT * FROM retired
    UNION ALL
    SELECT * FROM initials
    UNION ALL
    SELECT i.tenant_id, i.insight_source_id, i.issue_id, i.id_readable, concat('initial:', i.issue_id), i.created_at,
        'synthetic_initial', toUInt32(0), i.reporter_id, 'created', 'Created', 'single',
        CAST([] AS Array(Tuple(String, String))), i.observed_at, 'set'
    FROM {{ ref('youtrack__issues') }} AS i
), flagged_rows AS (
    SELECT *, (tenant_id, insight_source_id, field_id) IN (
            SELECT tenant_id, insight_source_id, field_id FROM {{ ref('youtrack__long_text_fields') }}) AS is_long_text
    FROM rows
), referenced_rows AS (
    -- A long-text value is a text reference: the body is in youtrack__task_field_text.
    SELECT * REPLACE (if(is_long_text,
            arrayMap(p -> ({{ task_text_id('p.2') }}, {{ task_text_prefix('p.2') }}), arrayFilter(p -> p.2 != '', pairs)),
            pairs) AS pairs)
    FROM flagged_rows
), field_types AS (
    SELECT tenant_id, insight_source_id, field_id,
        if(uniqExact(value_type) = 1, any(value_type), 'none') AS value_id_type
    FROM (
        SELECT tenant_id, insight_source_id, field_id, value_types FROM {{ ref('youtrack__activities') }}
        UNION ALL
        SELECT tenant_id, insight_source_id, field_id, value_types FROM {{ ref('youtrack__issue_field_snapshot') }}
    ) ARRAY JOIN value_types AS value_type
    GROUP BY tenant_id, insight_source_id, field_id
), ordered_rows AS (
    SELECT *, row_number() OVER w AS position,
        lagInFrame(pairs) OVER w AS previous_pairs,
        lagInFrame(field_cardinality) OVER w AS previous_cardinality
    FROM referenced_rows
    WINDOW w AS (PARTITION BY tenant_id, insight_source_id, issue_id, field_id
        ORDER BY event_at, {{ task_event_band('event_kind') }}, _seq
        ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW)
)
SELECT concat(ifNull(h.tenant_id, ''), '-', h.insight_source_id, '-youtrack-', h.issue_id, '-', h.field_id, '-',
              h.event_id) AS unique_key,
    ifNull(h.tenant_id, '') AS tenant_id,
    h.insight_source_id AS insight_source_id, 'youtrack' AS data_source, h.issue_id AS issue_id, h.id_readable AS id_readable, h.event_id AS event_id, h.event_at AS event_at,
    CAST(h.event_kind AS LowCardinality(String)) AS event_kind, h._seq AS _seq,
    {{ task_event_order("if(h.event_kind = 'synthetic_initial', least(h.event_at, ifNull(o.first_event_at, h.event_at)), h.event_at)", task_event_band('h.event_kind'), 'h._seq') }} AS event_order,
    h.author_id AS author_id, h.field_id AS field_id, h.field_name AS field_name,
    -- A retired field carries no value to read a cardinality from; it keeps the
    -- one its own field last had, so a field reads as one cardinality.
    CAST(if(h.event_kind = 'retired_field' AND h.previous_cardinality != '', h.previous_cardinality, h.field_cardinality)
        AS LowCardinality(String)) AS field_cardinality,
    CAST(h.delta_action AS LowCardinality(String)) AS delta_action,
    arrayMap(x -> x.1, h.pairs) AS value_ids, arrayMap(x -> x.2, h.pairs) AS value_displays,
    CAST(if(h.is_long_text, {{ task_text_ref_type() }}, ifNull(nullIf(t.value_id_type, ''), 'none')) AS LowCardinality(String)) AS value_id_type,
    h.collected_at AS collected_at, toUInt64(toUnixTimestamp64Milli(now64(3))) AS _version
FROM ordered_rows AS h
LEFT JOIN origins AS o
    ON o.tenant_id = h.tenant_id AND o.insight_source_id = h.insight_source_id AND o.issue_id = h.issue_id
LEFT JOIN field_types AS t
    ON t.tenant_id = h.tenant_id AND t.insight_source_id = h.insight_source_id AND t.field_id = h.field_id
WHERE throwIf(h._seq >= 100000, 'YouTrack event_order rank overflow') = 0
  AND (h.event_kind != 'snapshot_diff' OR h.position = 1 OR h.pairs != h.previous_pairs)
