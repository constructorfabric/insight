{{ config(
    materialized='ephemeral',
    tags=['gold']
) }}

-- Which vendor field plays which metric role, resolved to one row per
-- (source, field). Gold matches a role, never a vendor's field identifier —
-- `field_id` is documented as vendor-specific, so a literal like `'status'` is
-- Jira's name for the thing and no other source can honour it.
--
-- A vendor whose system fields are the same in every installation carries a
-- built-in default, so an operator writes nothing until a custom field needs a
-- role. A vendor that states no roles anywhere — GitHub, whose issue fields are
-- defined per organization — has no defaults and must be configured. An
-- authored row always wins over a default: that is the override channel for a
-- field whose vendor meaning does not match how a team uses it.
--
-- Ephemeral because it is small enough to inline wherever it is joined.
--
-- Resolution is as of the build, not as of the event: the row in force now
-- applies to all history. That is the correction semantics — an operator fixing
-- a wrong binding expects the fix to reach the numbers already published. The
-- validity axis is recorded and becomes effective when the class dimensions
-- gain a period grain; see the design's configuration section.
--
-- `tenant_id` is deliberately absent from the output. `class_task_field_history`
-- carries no tenant column — a source instance belongs to exactly one tenant,
-- so the source identifier already answers for it.

WITH
observed_sources AS (
    SELECT DISTINCT
        insight_source_id,
        data_source
    FROM {{ ref('class_task_field_history') }}
),

-- Jira's system field identifiers are fixed by the product, not by the
-- installation, so the mapping is the same everywhere and is stated here
-- rather than asked of every deployment. Time is already seconds.
vendor_defaults AS (
    SELECT
        s.insight_source_id                     AS insight_source_id,
        s.data_source                           AS data_source,
        d.1                                     AS field_id,
        d.2                                     AS role,
        d.3                                     AS value_unit
    FROM observed_sources AS s
    ARRAY JOIN [
        ('status',               'status',    'none'),
        ('assignee',             'assignee',  'none'),
        ('issuetype',            'issuetype', 'none'),
        ('duedate',              'duedate',   'none'),
        ('timeoriginalestimate', 'estimate',  'seconds'),
        ('timespent',            'spent',     'seconds')
    ] AS d
    WHERE s.data_source = 'jira'
),

authored AS (
    SELECT
        insight_source_id,
        data_source,
        field_id,
        argMax(role, (valid_from, recorded_at))            AS role,
        argMax(precedence, (valid_from, recorded_at))      AS precedence,
        argMax(value_unit, (valid_from, recorded_at))      AS value_unit,
        argMax(unit_multiplier, (valid_from, recorded_at)) AS unit_multiplier
    FROM {{ source('config', 'task_field_roles') }} FINAL
    WHERE is_deleted = 0
      AND valid_from <= now64(3)
    GROUP BY insight_source_id, data_source, field_id
),

every_binding AS (
    SELECT
        insight_source_id,
        data_source,
        field_id,
        CAST(role AS String)        AS role,
        precedence                  AS precedence,
        CAST(value_unit AS String)  AS value_unit,
        unit_multiplier             AS unit_multiplier
    FROM authored

    UNION ALL

    SELECT
        d.insight_source_id,
        d.data_source,
        d.field_id,
        CAST(d.role AS String)       AS role,
        toUInt8(0)                   AS precedence,
        CAST(d.value_unit AS String) AS value_unit,
        toFloat64(1)                 AS unit_multiplier
    FROM vendor_defaults AS d
    LEFT ANTI JOIN authored AS a
        ON a.insight_source_id = d.insight_source_id
        AND a.data_source = d.data_source
        AND a.field_id = d.field_id
)

SELECT *
FROM every_binding
WHERE role != 'ignored'
