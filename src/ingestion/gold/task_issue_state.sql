{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['insight_source_id', 'issue_id'],
    schema=var('gold_database'),
    alias='task_issue_state',
    tags=['gold'],
    query_settings={
        'max_memory_usage': 3221225472,
        'max_threads': 4,
        'max_bytes_before_external_group_by': 805306368,
        'max_bytes_before_external_sort': 805306368,
        'join_algorithm': 'grace_hash,hash'
    }
) }}

-- One row per assignee-resolved issue: attribution, current status category,
-- close time, and the scalar fields the observation measures read.
-- Materialized so the field-history pivot runs once per build instead of once
-- per measure branch (ClickHouse re-inlines every WITH reference).
--
-- Lifecycle comes from class_task_statuses.status_category ('done' = closed)
-- joined on the status id — never match status display names; issue type is the
-- same shape, via class_task_issuetypes.issue_kind.
-- Fields are matched by ROLE, not by vendor field id: `field_id` is documented
-- as vendor-specific, so a literal here would only ever be Jira's name for the
-- thing. `task_field_roles_current` carries the binding. Attribution:
-- assignee account id → lowercased email via class_task_users; only
-- email-shaped keys pass (unresolvable accounts are excluded, not carried).
-- Class reads keep FINAL: RMT parts are not duplicate-immune and argMax over
-- a stale version would skew the pivot.

WITH
task_users AS (
    SELECT
        tenant_id,
        insight_source_id,
        user_id,
        lower(email) AS email
    FROM {{ ref('class_task_users') }} FINAL
    WHERE email LIKE '%@%'
),
-- Per-issue scalar pivot; created = first synthetic_initial event.
history AS (
    SELECT
        fh.insight_source_id                                                  AS insight_source_id,
        fh.data_source                                                        AS data_source,
        fh.issue_id                                                           AS issue_id,
        fh.id_readable                                                        AS id_readable,
        fh.title                                                              AS title,
        fh.event_at                                                           AS event_at,
        fh.event_kind                                                         AS event_kind,
        fh.delta_action                                                       AS delta_action,
        fh.value_ids                                                          AS value_ids,
        fh.value_displays                                                     AS value_displays,
        fh._version                                                           AS _version,
        -- Null-proof under EITHER join_use_nulls setting: an unbound field must
        -- read as "no role", never as NULL propagating through the filter.
        -- `availability` is a contract sentinel (the jira deletion spec), not a
        -- vendor field — it gets its role directly, no binding row required.
        multiIf(fh.field_id = 'availability', 'availability',
                ifNull(r.role, ''))                                           AS role,
        -- Only a convertible unit is scaled. An estimate stated in a unit that
        -- is not commensurable with time must produce nothing rather than a
        -- plausible number.
        if(r.value_unit IN ('seconds', 'minutes', 'hours', 'days', 'man_days'),
           ifNull(r.unit_multiplier, 1), CAST(NULL AS Nullable(Float64)))     AS unit_multiplier
    FROM {{ ref('class_task_field_history') }} AS fh FINAL
    LEFT JOIN {{ ref('task_field_roles_current') }} AS r
        ON r.insight_source_id = fh.insight_source_id
        AND r.data_source = fh.data_source
        AND r.field_id = fh.field_id
    -- `availability` carries its role directly (above), so it must survive this
    -- filter too — r.role is NULL for it, being a contract sentinel with no
    -- vendor binding row.
    WHERE ifNull(r.role, '') != ''
       OR fh.field_id = 'availability'
       OR fh.event_kind = 'synthetic_initial'
),
issue_pivot AS (
    SELECT
        insight_source_id,
        issue_id,
        argMaxIf(value_ids[1], (event_at, _version),
                 role = 'status' AND delta_action = 'set')               AS status_id,
        argMaxIf(value_ids[1], (event_at, _version),
                 role = 'assignee' AND delta_action = 'set')             AS assignee_account_id,
        argMaxIf(value_displays[1], (event_at, _version),
                 role = 'issuetype' AND delta_action = 'set')            AS issue_type,
        argMaxIf(value_ids[1], (event_at, _version),
                 role = 'issuetype' AND delta_action = 'set')            AS issue_type_id,
        argMaxIf(value_displays[1], (event_at, _version),
                 role = 'duedate' AND delta_action = 'set')              AS due_date_str,
        toFloat64OrNull(argMaxIf(value_displays[1], (event_at, _version),
                 role = 'estimate' AND delta_action = 'set'))
            * argMaxIf(unit_multiplier, (event_at, _version),
                 role = 'estimate' AND delta_action = 'set')                 AS time_estimate_seconds,
        toFloat64OrNull(argMaxIf(value_displays[1], (event_at, _version),
                 role = 'spent' AND delta_action = 'set'))
            * argMaxIf(unit_multiplier, (event_at, _version),
                 role = 'spent' AND delta_action = 'set')                    AS time_spent_seconds,
        minIf(event_at, event_kind = 'synthetic_initial')                    AS created_at,
        -- The key the tracker itself shows a human ('owner/repo#12', 'PROJ-7');
        -- the only field an issue's own page can be addressed from.
        -- INVARIANT: argMax, never any() — `id_readable` is part of `unique_key`,
        -- so a renamed repository or an issue moved between projects leaves rows
        -- under BOTH keys and FINAL collapses neither. The latest event wins.
        argMax(id_readable, (event_at, _version))                            AS id_readable,
        argMax(title, (event_at, _version))                                  AS title,
        maxIf(event_at, role = 'status' AND delta_action = 'set')        AS last_status_event_at,
        -- Availability lives in the same history as every other field
        -- (synthetic 'availability' events; see the jira deletion spec).
        argMaxIf(value_ids[1], (event_at, _version),
                 role = 'availability')                                      AS availability,
        any(data_source)                                                     AS data_source
    FROM history
    GROUP BY insight_source_id, issue_id
),
-- Close time: the last transition into a done-category status. OrNull so a
-- never-closed issue is NULL, not the epoch default of the non-Nullable
-- event_at — `final_close_at IS NOT NULL` gates the closed-issue measures.
issue_close AS (
    SELECT
        fh.insight_source_id                                                 AS insight_source_id,
        fh.issue_id                                                          AS issue_id,
        maxIfOrNull(fh.event_at, st.status_category = 'done')                AS final_close_at
    FROM history AS fh
    LEFT JOIN {{ ref('class_task_statuses') }} AS st FINAL
        ON st.insight_source_id = fh.insight_source_id
        AND st.status_id = fh.value_ids[1]
    WHERE fh.role = 'status' AND fh.delta_action = 'set'
    GROUP BY fh.insight_source_id, fh.issue_id
)
SELECT
    u.tenant_id                                                              AS tenant_id,
    u.email                                                                  AS entity_id,
    p.insight_source_id                                                      AS insight_source_id,
    p.data_source                                                            AS data_source,
    p.issue_id                                                               AS issue_id,
    p.id_readable                                                            AS id_readable,
    p.title                                                                  AS title,
    cur.status_category                                                      AS status_category,
    p.issue_type                                                             AS issue_type,
    ifNull(it.issue_kind, 'unknown')                                         AS issue_kind,
    coalesce(it.untranslated_name, it.issue_type_name, nullIf(p.issue_type, '')) AS issue_type_key,
    coalesce(it.issue_type_name, nullIf(p.issue_type, ''))                   AS issue_type_name,
    if(p.due_date_str IS NOT NULL AND p.due_date_str != '',
       toDate(parseDateTimeBestEffortOrNull(p.due_date_str)),
       CAST(NULL AS Nullable(Date)))                                         AS due_date,
    p.time_estimate_seconds                                                  AS time_estimate_seconds,
    p.time_spent_seconds                                                     AS time_spent_seconds,
    p.created_at                                                             AS created_at,
    c.final_close_at                                                         AS final_close_at,
    p.last_status_event_at                                                   AS last_status_event_at
FROM issue_pivot AS p
INNER JOIN task_users AS u
    ON u.insight_source_id = p.insight_source_id
    AND u.user_id = p.assignee_account_id
LEFT JOIN issue_close AS c
    ON c.insight_source_id = p.insight_source_id AND c.issue_id = p.issue_id
LEFT JOIN {{ ref('class_task_statuses') }} AS cur FINAL
    ON cur.insight_source_id = p.insight_source_id AND cur.status_id = p.status_id
LEFT JOIN {{ ref('class_task_issuetypes') }} AS it FINAL
    ON it.insight_source_id = p.insight_source_id AND it.issue_type_id = p.issue_type_id
-- Issues deleted at the source (or in the project trash) leave every task
-- metric: this table is the root of the gold task chain, so the filter
-- propagates to spans, worklog flow and evidence. archived / access_lost /
-- unobserved issues stay in — the entity still exists, its data is merely
-- stale. Issues with no availability events default to present ('').
WHERE p.availability NOT IN ('deleted', 'trashed')
