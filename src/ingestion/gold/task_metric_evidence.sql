{{ metric_evidence_table(query_settings_overrides={'max_memory_usage': 3221225472}) }}

{% set account_keys = ['account_source_type', 'account_source_id', 'account_id'] %}

-- Keyed by the source identity, not by person: the tracker account of the
-- assignee or worklog author, plus its email when the tracker states one. The
-- analytics runtime resolves account-first through `identity.account_assignment`
-- and falls back to `identity.person_map` while it serves. An unresolvable row
-- stays and starts counting the moment it resolves; until then it, and every
-- unassigned issue's row, counts only in the unattributed share.
SELECT
    src.tenant_id,
    src.source_key,
    src.entity_type,
    {{ normalized_email('src.entity_id') }} AS entity_id,
    -- INVARIANT: every account column is a plain String in every evidence
    -- relation; the service's exact-column probe blanks a family that differs.
    CAST(coalesce(src.account_source_type, '') AS String) AS account_source_type,
    CAST(coalesce(src.account_source_id, '') AS String) AS account_source_id,
    CAST(coalesce(src.account_id, '') AS String) AS account_id,
    src.metric_date,
    src.observed_at,
    src.measure_key,
    src.record_id,
    src.record_kind,
    src.granularity,
    src.record_label,
    src.contribution,
    src.subject_key,
    src.dimensions,
    src.details
FROM (


WITH
issue_state AS (
    SELECT *
    FROM {{ ref('task_issue_state') }}
),
status_intervals AS (
    SELECT *
    FROM {{ ref('task_status_spans') }}
),
issue_facts AS (
    SELECT
        s.tenant_id                                                          AS tenant_id,
        s.entity_id                                                          AS entity_id,
        s.account_source_type                                                AS account_source_type,
        s.account_source_id                                                  AS account_source_id,
        s.account_id                                                         AS account_id,
        s.insight_source_id                                                   AS insight_source_id,
        toDate(s.final_close_at)                                             AS metric_date,
        any(s.final_close_at)                                                AS observed_at,
        s.issue_id                                                           AS issue_id,
        any(s.data_source)                                                   AS data_source,
        any(s.id_readable)                                                   AS id_readable,
        any(s.title)                                                         AS title,
        any(s.status_category) = 'done'                                      AS is_done,
        toDate(s.final_close_at)                                             AS close_date,
        any(s.due_date)                                                      AS due_date,
        any(s.time_estimate_seconds)                                         AS time_estimate_seconds,
        any(s.time_spent_seconds)                                            AS time_spent_seconds,
        sumIf(i.duration_seconds, i.interval_start < s.final_close_at)      AS dev_seconds,
        if(any(s.created_at) IS NULL,
           CAST(NULL AS Nullable(Float64)),
           toFloat64(greatest(toInt64(0),
               dateDiff('second', any(s.created_at), any(s.final_close_at))))) AS lead_seconds,
        -- INVARIANT: an issue with no in-progress span joins one default row whose
        -- start is 1970-01-01; matching the category keeps it out of the minimum.
        if(any(s.created_at) IS NULL
               OR minIfOrNull(i.interval_start, i.status_category = 'in_progress' AND i.interval_start < s.final_close_at) IS NULL,
           CAST(NULL AS Nullable(Float64)),
           toFloat64(greatest(toInt64(0),
               dateDiff('second', any(s.created_at),
                        minIfOrNull(i.interval_start, i.status_category = 'in_progress' AND i.interval_start < s.final_close_at))))) AS pickup_seconds,
        -- The flow and estimation measures taken off these rows carry no
        -- breakdown, but a row still has to name its tracker: `source` is what
        -- makes its ref addressable.
        CAST(
            [tuple('source', any(s.data_source), any(s.data_source))]
            AS Array(Tuple(key String, value String, label Nullable(String)))
        ) AS source_dimensions,
        -- The per-issue duration rows carry the issue's own type as well, so a
        -- median reads the same way a count of closed issues does: which kinds
        -- of work it was taken over.
        CAST(
            [
                tuple(
                    'type',
                    ifNull(any(s.issue_type_key), '__unknown__'),
                    ifNull(any(s.issue_type_name), 'Type unknown')
                ),
                tuple('source', any(s.data_source), any(s.data_source)),
                -- The kind is the label too: the domain is bounded, and a
                -- vendor name would fragment one class into many groups.
                tuple('resolution', any(s.resolution_kind), any(s.resolution_kind))
            ] AS Array(Tuple(key String, value String, label Nullable(String)))
        ) AS type_dimensions
    FROM issue_state AS s
    LEFT JOIN status_intervals AS i
        ON i.insight_source_id = s.insight_source_id
        AND i.issue_id = s.issue_id
        AND i.status_category = 'in_progress'
    WHERE s.final_close_at IS NOT NULL
    GROUP BY s.tenant_id, s.entity_id, s.account_source_type, s.account_source_id, s.account_id,
             s.insight_source_id, s.issue_id, toDate(s.final_close_at)
),
issue_item_evidence AS (
    SELECT
        tenant_id,
        entity_id,
        account_source_type,
        account_source_id,
        account_id,
        insight_source_id,
        toDate(final_close_at) AS metric_date,
        final_close_at AS observed_at,
        issue_id,
        id_readable,
        title,
        item_measure.1 AS measure_key,
        toFloat64(item_measure.2) AS contribution,
        CAST(
            [
                tuple(
                    'type',
                    ifNull(issue_type_key, '__unknown__'),
                    ifNull(issue_type_name, 'Type unknown')
                ),
                -- Without this, two trackers blend into one per-person figure
                -- with no way to tell them apart, and an issue mirrored between
                -- them is counted twice with nothing to say so.
                tuple('source', data_source, data_source),
                tuple('resolution', resolution_kind, resolution_kind)
            ] AS Array(Tuple(key String, value String, label Nullable(String)))
        ) AS type_dimensions
    FROM issue_state
    ARRAY JOIN arrayConcat(
        [tuple('tasks_closed', toFloat64(1))],
        if(issue_kind = 'bug', [tuple('bugs_fixed', toFloat64(1))], []),
        if(issue_kind = 'task', [tuple('closed_task', toFloat64(1))], []),
        if(issue_kind = 'unknown', [tuple('closed_unknown_type', toFloat64(1))], []),
        -- Closed-by-resolution-class subsets of tasks_closed. The unknown
        -- class deliberately has no measure: the unclassified-closures alert
        -- owns it.
        if(resolution_kind = 'fixed', [tuple('closed_fixed', toFloat64(1))], []),
        if(resolution_kind = 'duplicate', [tuple('closed_duplicate', toFloat64(1))], []),
        if(resolution_kind = 'wontfix', [tuple('closed_wontfix', toFloat64(1))], []),
        if(
            due_date IS NOT NULL AND toDate(final_close_at) <= due_date,
            [tuple('due_date_on_time', toFloat64(1))],
            []
        ),
        if(due_date IS NOT NULL, [tuple('due_date_with_due', toFloat64(1))], []),
        if(
            due_date IS NOT NULL AND toDate(final_close_at) > due_date,
            [
                tuple(
                    'slip_days_total',
                    toFloat64(dateDiff('day', due_date, toDate(final_close_at)))
                ),
                tuple('late_count', toFloat64(1))
            ],
            []
        )
    ) AS item_measure
    WHERE final_close_at IS NOT NULL
      AND status_category = 'done'
),
-- A day's estimate deviation is not additive across accounts, so it is taken
-- per person-day (`task_person_day_group`), the grain the email gave it.
estimation_rows AS (
    SELECT
        tenant_id                                                            AS day_tenant_id,
        entity_id                                                            AS day_entity_id,
        account_source_type                                                  AS day_account_source_type,
        account_source_id                                                    AS day_account_source_id,
        account_id                                                           AS day_account_id,
        {{ task_person_day_group('') }}                                      AS person_day,
        metric_date                                                          AS day_metric_date,
        observed_at,
        insight_source_id,
        issue_id,
        data_source,
        id_readable,
        title,
        is_done AND ifNull(time_estimate_seconds, 0) > 0 AND time_spent_seconds IS NOT NULL AS estimated,
        time_estimate_seconds,
        time_spent_seconds
    FROM issue_facts
),
-- INVARIANT: each estimated issue holds an equal share of its person-day's
-- deviation and of its one sample, so the shares sum back to the day's figure.
estimation_issues AS (
    SELECT
        day_tenant_id AS tenant_id,
        day_entity_id AS entity_id,
        {{ task_person_day_account_columns('day_', window='person_day_window') }},
        day_metric_date AS metric_date,
        observed_at,
        insight_source_id,
        issue_id,
        data_source,
        id_readable,
        title,
        estimated,
        100 * avgIf(time_estimate_seconds, estimated) OVER person_day_window
            / nullIf(avgIf(time_spent_seconds, estimated) OVER person_day_window, 0)
            AS estimation_pct,
        countIf(estimated) OVER person_day_window AS estimated_issues
    FROM estimation_rows
    WINDOW person_day_window AS (PARTITION BY day_tenant_id, day_entity_id, person_day, day_metric_date)
),
transitions AS (
    SELECT
        insight_source_id,
        issue_id,
        interval_start AS event_at,
        status_category,
        lagInFrame(status_category) OVER (
            PARTITION BY insight_source_id, issue_id ORDER BY interval_start
        ) AS prev_category
    FROM status_intervals
),
closes AS (
    SELECT insight_source_id, issue_id, event_at AS close_at
    FROM transitions
    WHERE status_category = 'done' AND (prev_category IS NULL OR prev_category != 'done')
),
reopens AS (
    SELECT insight_source_id, issue_id, event_at AS reopen_at
    FROM transitions
    WHERE prev_category = 'done' AND (status_category != 'done' OR status_category IS NULL)
),
close_reopen AS (
    SELECT
        s.tenant_id                                                          AS tenant_id,
        s.entity_id                                                          AS entity_id,
        s.account_source_type                                                AS account_source_type,
        s.account_source_id                                                  AS account_source_id,
        s.account_id                                                         AS account_id,
        c.insight_source_id                                                  AS insight_source_id,
        c.issue_id                                                           AS issue_id,
        any(s.data_source)                                                   AS data_source,
        any(s.id_readable)                                                   AS id_readable,
        any(s.title)                                                         AS title,
        c.close_at                                                           AS close_at,
        -- OrNull, not minIf: over a non-Nullable column with nothing matching,
        -- `minIf` returns the type's default — 1970-01-01, which IS NOT NULL —
        -- so every close of every issue read as reopened. Only a fixture with a
        -- reopened close AND a clean one alongside it shows the difference.
        if(minIfOrNull(r.reopen_at, r.reopen_at > c.close_at) IS NOT NULL
           AND minIfOrNull(r.reopen_at, r.reopen_at > c.close_at) <= c.close_at + INTERVAL 14 DAY,
           toFloat64(1), toFloat64(0))                                       AS reopened_14d
    FROM closes AS c
    INNER JOIN issue_state AS s
        ON s.insight_source_id = c.insight_source_id AND s.issue_id = c.issue_id
    LEFT JOIN reopens AS r
        ON r.insight_source_id = c.insight_source_id AND r.issue_id = c.issue_id
    GROUP BY s.tenant_id, s.entity_id, s.account_source_type, s.account_source_id, s.account_id,
             c.insight_source_id, c.issue_id, c.close_at
),
issue_measures AS (
    SELECT
        tenant_id,
        entity_id,
        account_source_type,
        account_source_id,
        account_id,
        metric_date,
        toNullable(toDateTime64(observed_at, 3)) AS observed_at,
        insight_source_id,
        issue_id,
        data_source,
        id_readable,
        title,
        estimation_measure.1 AS measure_key,
        estimation_measure.2 AS contribution
    FROM estimation_issues
    ARRAY JOIN [
        tuple('estimation_error_pct', abs(100 - estimation_pct) / estimated_issues),
        tuple('estimation_samples', 1 / estimated_issues)
    ] AS estimation_measure
    WHERE estimated
      AND estimation_pct > 0
      AND estimation_pct <= 200

    UNION ALL

    SELECT
        c.tenant_id AS tenant_id,
        c.entity_id AS entity_id,
        c.account_source_type AS account_source_type,
        c.account_source_id AS account_source_id,
        c.account_id AS account_id,
        toDate(c.close_at) AS metric_date,
        toNullable(toDateTime64(max(c.close_at), 3)) AS observed_at,
        c.insight_source_id AS insight_source_id,
        c.issue_id AS issue_id,
        any(c.data_source) AS data_source,
        any(c.id_readable) AS id_readable,
        any(c.title) AS title,
        close_measure.1 AS measure_key,
        sum(close_measure.2) AS contribution
    FROM close_reopen AS c
    ARRAY JOIN arrayConcat(
        [tuple('close_events', toFloat64(1))],
        if(c.reopened_14d > 0, [tuple('reopened_within_14d', c.reopened_14d)], [])
    ) AS close_measure
    GROUP BY c.tenant_id, c.entity_id, c.account_source_type, c.account_source_id, c.account_id,
             c.insight_source_id, c.issue_id, toDate(c.close_at), close_measure.1

    UNION ALL

    -- INVARIANT: zero rows stay. Without them a person-day that logged
    -- nothing reads no data instead of 0%.
    SELECT
        tenant_id,
        entity_id,
        account_source_type,
        account_source_id,
        account_id,
        metric_date,
        CAST(NULL AS Nullable(DateTime64(3))) AS observed_at,
        insight_source_id,
        issue_id,
        data_source,
        id_readable,
        title,
        flow_measure.1 AS measure_key,
        flow_measure.2 AS contribution
    FROM {{ ref('task_worklog_flow') }}
    ARRAY JOIN [
        tuple('worklog_seconds', worklog_seconds),
        tuple('in_progress_seconds', in_progress_seconds)
    ] AS flow_measure
    WHERE person_day_in_progress_seconds > 0

    UNION ALL

    SELECT
        tenant_id,
        entity_id,
        account_source_type,
        account_source_id,
        account_id,
        toDate(last_status_event_at) AS metric_date,
        toNullable(toDateTime64(last_status_event_at, 3)) AS observed_at,
        insight_source_id,
        issue_id,
        data_source,
        id_readable,
        title,
        'stale_in_progress' AS measure_key,
        toFloat64(1) AS contribution
    FROM issue_state
    WHERE (status_category IS NULL OR status_category != 'done')
      AND last_status_event_at IS NOT NULL
      AND dateDiff('day', last_status_event_at, now()) > 14
),
-- Account grain, not the person-day of worklog_flow: a sum needs no gate, so
-- each account's hours keep its key and resolve account-first.
logged_work AS (
    SELECT
        tenant_id,
        entity_id,
        account_source_type,
        account_source_id,
        account_id,
        metric_date,
        worklog_seconds,
        CAST([] AS Array(Tuple(key String, value String, label Nullable(String)))) AS no_dimensions
    FROM ({{ task_worklog_per_account_day() }})
)
SELECT
    assumeNotNull(tenant_id) AS tenant_id,
    'task' AS source_key,
    'person' AS entity_type,
    assumeNotNull(entity_id) AS entity_id,
    account_source_type,
    account_source_id,
    account_id,
    assumeNotNull(metric_date) AS metric_date,
    observed_at,
    measure_key,
    concat(
        toString(insight_source_id),
        ':',
        toString(issue_id),
        ':',
        measure_key,
        ':',
        -- INVARIANT: the account is part of the key. Email-less accounts share
        -- entity_id '', so two of them on one issue collide without it.
        hex(sipHash128(
            coalesce(account_source_type, ''),
            coalesce(account_source_id, ''),
            coalesce(account_id, '')
        ))
    ) AS record_id,
    'issue' AS record_kind,
    'event' AS granularity,
    id_readable AS record_label,
    toNullable(contribution) AS contribution,
    CAST(NULL AS Nullable(String)) AS subject_key,
    CAST(
        [tuple('source', data_source, data_source)]
        AS Array(Tuple(key String, value String, label Nullable(String)))
    ) AS dimensions,
    map(
        'source_id', toString(insight_source_id),
        'ref', id_readable,
        'title', ifNull(title, '')
    ) AS details
FROM issue_measures
WHERE tenant_id IS NOT NULL
  AND entity_id IS NOT NULL
  AND metric_date IS NOT NULL

UNION ALL

SELECT
    assumeNotNull(tenant_id) AS tenant_id,
    'task' AS source_key,
    'person' AS entity_type,
    assumeNotNull(entity_id) AS entity_id,
    account_source_type,
    account_source_id,
    account_id,
    assumeNotNull(metric_date) AS metric_date,
    toNullable(toDateTime64(observed_at, 3)) AS observed_at,
    measure_key,
    concat(toString(insight_source_id), ':', toString(issue_id), ':', measure_key) AS record_id,
    'issue' AS record_kind,
    'event' AS granularity,
    id_readable AS record_label,
    toNullable(contribution) AS contribution,
    CAST(NULL AS Nullable(String)) AS subject_key,
    type_dimensions AS dimensions,
    map(
        'source_id', toString(insight_source_id),
        'ref', id_readable,
        'title', ifNull(title, ''),
        'type', ifNull(arrayFirst(dimension -> dimension.1 = 'type', type_dimensions).3, '')
    ) AS details
FROM issue_item_evidence
WHERE tenant_id IS NOT NULL
  AND entity_id IS NOT NULL
  AND metric_date IS NOT NULL

UNION ALL

SELECT
    assumeNotNull(tenant_id) AS tenant_id,
    'task' AS source_key,
    'person' AS entity_type,
    assumeNotNull(entity_id) AS entity_id,
    account_source_type,
    account_source_id,
    account_id,
    assumeNotNull(metric_date) AS metric_date,
    toNullable(toDateTime64(observed_at, 3)) AS observed_at,
    issue_measure.1 AS measure_key,
    concat(toString(insight_source_id), ':', toString(issue_id), ':', issue_measure.1) AS record_id,
    'issue' AS record_kind,
    'event' AS granularity,
    id_readable AS record_label,
    toNullable(toFloat64(issue_measure.2)) AS contribution,
    CAST(NULL AS Nullable(String)) AS subject_key,
    issue_measure.3 AS dimensions,
    map(
        'source_id', toString(insight_source_id),
        'ref', id_readable,
        'title', ifNull(title, '')
    ) AS details
FROM issue_facts
ARRAY JOIN arrayConcat(
    if(ifNull(lead_seconds, 0) > 0, [tuple('resolution_days', toFloat64(lead_seconds / 86400.0), type_dimensions)], []),
    if(pickup_seconds IS NOT NULL, [tuple('pickup_days', toFloat64(pickup_seconds / 86400.0), type_dimensions)], []),
    if(
        ifNull(dev_seconds, 0) > 0 AND ifNull(lead_seconds, 0) > 0,
        [
            tuple('flow_dev_seconds', toFloat64(dev_seconds), source_dimensions),
            tuple('flow_lead_seconds', toFloat64(lead_seconds), source_dimensions)
        ],
        []
    )
) AS issue_measure
WHERE tenant_id IS NOT NULL
  AND entity_id IS NOT NULL
  AND metric_date IS NOT NULL

UNION ALL

SELECT
    assumeNotNull(tenant_id) AS tenant_id,
    'task' AS source_key,
    'person' AS entity_type,
    assumeNotNull(entity_id) AS entity_id,
    account_source_type,
    account_source_id,
    account_id,
    assumeNotNull(metric_date) AS metric_date,
    CAST(NULL AS Nullable(DateTime64(3))) AS observed_at,
    measure_key,
    concat(
        toString(metric_date),
        ':',
        measure_key,
        ':',
        -- INVARIANT: the account is part of the key. Email-less accounts share
        -- entity_id '', so without it their day rows collide on one record_id.
        hex(sipHash128(
            toString(arrayMap(d -> tuple(d.1, d.2), dimensions)),
            coalesce(account_source_type, ''),
            coalesce(account_source_id, ''),
            coalesce(account_id, '')
        ))
    ) AS record_id,
    measure_key AS record_kind,
    'derived_population' AS granularity,
    replaceAll(measure_key, '_', ' ') AS record_label,
    value AS contribution,
    CAST(NULL AS Nullable(String)) AS subject_key,
    dimensions,
    CAST(map() AS Map(String, String)) AS details
FROM ({{ sum_measure('logged_hours', 'logged_work', 'worklog_seconds / 3600.0', 'no_dimensions', where='worklog_seconds > 0', keys=account_keys) }})
WHERE tenant_id IS NOT NULL
  AND entity_id IS NOT NULL
  AND metric_date IS NOT NULL
) AS src
