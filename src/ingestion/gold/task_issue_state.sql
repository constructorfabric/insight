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
-- joined on the status id — never match status display names. The issue KIND
-- (bug / task / unknown) is resolved HERE, not in silver: the operator's
-- `config.field_value_map` row for (tenant, source, field='issue_type',
-- source_key = type id) decides it; an unmapped id falls to the tenant's
-- `config.field_value_defaults` row for the field, then to 'unknown' — never
-- to a name or the raw value. The RESOLUTION kind (fixed / duplicate /
-- wontfix / unknown) resolves the same way under field='resolution', except
-- an issue with no resolution value at all is 'unknown' outright — the
-- default speaks only for unmapped values, not absent ones. Resolving at the
-- gold build is what makes a mapping change apply on the next run without a
-- silver rebuild. `class_task_issuetypes` stays the raw catalogue dimension
-- for the type names.
-- Fields are matched by ROLE, not by vendor field id: `field_id` is documented
-- as vendor-specific, so a literal here would only ever be Jira's name for the
-- thing. `task_field_roles_current` carries the binding. Attribution:
-- assignee account id → lowercased email via class_task_users; only
-- email-shaped keys pass (unresolvable accounts are excluded, not carried).
-- Class reads keep FINAL: RMT parts are not duplicate-immune and argMax over
-- a stale version would skew the pivot.

WITH
-- Operator issue-type decisions: bitemporal latest row per key FIRST, the
-- tombstone/domain filter AFTER. INVARIANT: unique_key includes recorded_at,
-- so a retraction (is_deleted=1) or a typo is a NEWER separate row — filtering
-- before LIMIT 1 BY would resurrect the older live decision. When the newest
-- row is a tombstone or outside the domain, the key classifies as unmapped;
-- `assert_field_values_are_canonical` reports the typo row itself.
issue_type_map AS (
    SELECT
        tenant_id,
        insight_source_id,
        source_key,
        target_value
    FROM (
        SELECT
            tenant_id,
            insight_source_id,
            source_key,
            target_value,
            is_deleted
        FROM {{ source('config', 'field_value_map') }} FINAL
        WHERE field = 'issue_type'
          AND valid_from <= now64(3)
        ORDER BY valid_from DESC, recorded_at DESC
        LIMIT 1 BY tenant_id, insight_source_id, source_key
    )
    WHERE is_deleted = 0
      AND target_value IN ('bug', 'task', 'unknown')
),
-- Per (tenant, source) fallback for unmapped type ids; same shape, same guard.
issue_type_default AS (
    SELECT
        tenant_id,
        insight_source_id,
        default_value
    FROM (
        SELECT
            tenant_id,
            insight_source_id,
            default_value,
            is_deleted
        FROM {{ source('config', 'field_value_defaults') }} FINAL
        WHERE field = 'issue_type'
          AND valid_from <= now64(3)
        ORDER BY valid_from DESC, recorded_at DESC
        LIMIT 1 BY tenant_id, insight_source_id
    )
    WHERE is_deleted = 0
      AND default_value IN ('bug', 'task', 'unknown')
),
-- Operator resolution decisions; same bitemporal shape and domain guard as
-- the issue-type pair above. INVARIANT: all four decision CTEs stay in
-- lockstep — latest row first, tombstone/domain filter after.
resolution_map AS (
    SELECT
        tenant_id,
        insight_source_id,
        source_key,
        target_value
    FROM (
        SELECT
            tenant_id,
            insight_source_id,
            source_key,
            target_value,
            is_deleted
        FROM {{ source('config', 'field_value_map') }} FINAL
        WHERE field = 'resolution'
          AND valid_from <= now64(3)
        ORDER BY valid_from DESC, recorded_at DESC
        LIMIT 1 BY tenant_id, insight_source_id, source_key
    )
    WHERE is_deleted = 0
      AND target_value IN ('fixed', 'duplicate', 'wontfix', 'unknown')
),
resolution_default AS (
    SELECT
        tenant_id,
        insight_source_id,
        default_value
    FROM (
        SELECT
            tenant_id,
            insight_source_id,
            default_value,
            is_deleted
        FROM {{ source('config', 'field_value_defaults') }} FINAL
        WHERE field = 'resolution'
          AND valid_from <= now64(3)
        ORDER BY valid_from DESC, recorded_at DESC
        LIMIT 1 BY tenant_id, insight_source_id
    )
    WHERE is_deleted = 0
      AND default_value IN ('fixed', 'duplicate', 'wontfix', 'unknown')
),
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
        fh.event_at                                                           AS event_at,
        fh.event_kind                                                         AS event_kind,
        fh.delta_action                                                       AS delta_action,
        fh.value_ids                                                          AS value_ids,
        fh.value_displays                                                     AS value_displays,
        fh._version                                                           AS _version,
        -- Part of the ordering key below, not payload: see `task_event_rank`.
        fh._seq                                                               AS _seq,
        fh.event_id                                                           AS event_id,
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
        argMaxIf(value_ids[1], (event_at, {{ task_event_rank('event_kind') }}, _seq, toUInt64OrZero(event_id)),
                 role = 'status' AND delta_action = 'set')               AS status_id,
        argMaxIf(value_ids[1], (event_at, {{ task_event_rank('event_kind') }}, _seq, toUInt64OrZero(event_id)),
                 role = 'assignee' AND delta_action = 'set')             AS assignee_account_id,
        argMaxIf(value_displays[1], (event_at, {{ task_event_rank('event_kind') }}, _seq, toUInt64OrZero(event_id)),
                 role = 'issuetype' AND delta_action = 'set')            AS issue_type,
        argMaxIf(value_ids[1], (event_at, {{ task_event_rank('event_kind') }}, _seq, toUInt64OrZero(event_id)),
                 role = 'issuetype' AND delta_action = 'set')            AS issue_type_id,
        argMaxIf(value_ids[1], (event_at, {{ task_event_rank('event_kind') }}, _seq, toUInt64OrZero(event_id)),
                 role = 'resolution' AND delta_action = 'set')           AS resolution_id_raw,
        argMaxIf(value_displays[1], (event_at, {{ task_event_rank('event_kind') }}, _seq, toUInt64OrZero(event_id)),
                 role = 'duedate' AND delta_action = 'set')              AS due_date_str,
        toFloat64OrNull(argMaxIf(value_displays[1], (event_at, {{ task_event_rank('event_kind') }}, _seq, toUInt64OrZero(event_id)),
                 role = 'estimate' AND delta_action = 'set'))
            * argMaxIf(unit_multiplier, (event_at, {{ task_event_rank('event_kind') }}, _seq, toUInt64OrZero(event_id)),
                 role = 'estimate' AND delta_action = 'set')                 AS time_estimate_seconds,
        toFloat64OrNull(argMaxIf(value_displays[1], (event_at, {{ task_event_rank('event_kind') }}, _seq, toUInt64OrZero(event_id)),
                 role = 'spent' AND delta_action = 'set'))
            * argMaxIf(unit_multiplier, (event_at, {{ task_event_rank('event_kind') }}, _seq, toUInt64OrZero(event_id)),
                 role = 'spent' AND delta_action = 'set')                    AS time_spent_seconds,
        minIf(event_at, event_kind = 'synthetic_initial')                    AS created_at,
        -- The key the tracker itself shows a human ('owner/repo#12', 'PROJ-7');
        -- the only field an issue's own page can be addressed from.
        -- INVARIANT: argMax, never any() — a renamed repository or an issue moved
        -- between projects carries the OLD key on its older rows, and rows written
        -- before the key moved to `issue_id` exist under both. The latest event wins.
        argMax(id_readable, (event_at, {{ task_event_rank('event_kind') }}, _seq, toUInt64OrZero(event_id)))                            AS id_readable,
        -- The title is an ordinary field read through its role, so a source
        -- that renames an issue has rename history. `nullIf` keeps the result
        -- `Nullable(String)`: `argMaxIf` returns '' when nothing matches, and
        -- this is a serving table whose type the backend reads.
        nullIf(argMaxIf(value_displays[1], (event_at, {{ task_event_rank('event_kind') }}, _seq, toUInt64OrZero(event_id)),
                        role = 'title'), '')                                 AS title,
        maxIf(event_at, role = 'status' AND delta_action = 'set')        AS last_status_event_at,
        -- Availability lives in the same history as every other field
        -- (synthetic 'availability' events; see the jira deletion spec).
        argMaxIf(value_ids[1], (event_at, {{ task_event_rank('event_kind') }}, _seq, toUInt64OrZero(event_id)),
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
    -- Mapping row → tenant default → 'unknown'. A join miss reads as ''
    -- under join_use_nulls=0 (the non-Nullable String default) and NULL
    -- under =1 — nullIf folds both into the next fallback, so the chain is
    -- regime-independent. CAST off the LowCardinality the config columns
    -- carry: this is a serving table whose type the backend reads.
    CAST(coalesce(
        nullIf(toString(m.target_value), ''),
        nullIf(toString(d.default_value), ''),
        'unknown'
    ) AS String)                                                             AS issue_kind,
    coalesce(it.untranslated_name, nullIf(it.issue_type_name, ''),
             nullIf(p.issue_type, ''))                                       AS issue_type_key,
    coalesce(nullIf(it.issue_type_name, ''), nullIf(p.issue_type, ''))       AS issue_type_name,
    -- An issue that carries no resolution value is 'unknown' outright: the
    -- default row speaks for UNMAPPED values, and letting it claim absent
    -- ones would classify every open issue.
    if(nullIf(p.resolution_id_raw, '') IS NULL,
       'unknown',
       CAST(coalesce(
           nullIf(toString(rm.target_value), ''),
           nullIf(toString(rd.default_value), ''),
           'unknown'
       ) AS String))                                                         AS resolution_kind,
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
LEFT JOIN issue_type_map AS m
    ON m.tenant_id = u.tenant_id
    AND m.insight_source_id = p.insight_source_id
    AND m.source_key = p.issue_type_id
LEFT JOIN issue_type_default AS d
    ON d.tenant_id = u.tenant_id
    AND d.insight_source_id = p.insight_source_id
LEFT JOIN resolution_map AS rm
    ON rm.tenant_id = u.tenant_id
    AND rm.insight_source_id = p.insight_source_id
    AND rm.source_key = p.resolution_id_raw
LEFT JOIN resolution_default AS rd
    ON rd.tenant_id = u.tenant_id
    AND rd.insight_source_id = p.insight_source_id
-- Issues deleted at the source (or in the project trash) leave every task
-- metric: this table is the root of the gold task chain, so the filter
-- propagates to spans, worklog flow and evidence. archived / access_lost /
-- unobserved issues stay in — the entity still exists, its data is merely
-- stale. Issues with no availability events default to present ('').
WHERE p.availability NOT IN ('deleted', 'trashed')
