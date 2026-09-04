{{ config(severity='warn') }}

-- A field whose changelog ids and current-value ids are two disjoint spaces.
--
-- SEVERITY: warn. This test reports a condition of the SOURCE, not a fault of
-- the pipeline, and the condition cannot be repaired from here. A failing gate
-- that can never be made to pass stops being a signal and starts being noise
-- someone routes around — so it warns in the nightly run and the reasons are
-- enumerated in the spec. The transformation lane runs it at ERROR strictness
-- over controlled inputs, where a real regression WOULD be a pipeline fault.
--
-- FIELD-HISTORY-IN-DBT.md §3.4. Deleting and recreating a custom field context,
-- or deleting an option instead of disabling it, gives every option a new id
-- while the display text stays. Jira records no event for the change, so within
-- one field the ids the changelog names and the ids the issues now hold stop
-- intersecting, with no mapping between them anywhere in the data.
--
-- Nothing downstream can repair that. This test exists so it is NAMED before a
-- metric is bound to such a field: grouping history by option id then splits one
-- value in two at the boundary, and grouping by display text merges values that
-- really did change. Until a role points at the field the problem is latent.
--
-- Two guards keep it from crying wolf.
--
-- SCOPE. Only the kinds whose ids identify a value drawn from an administered,
-- reusable set: an option, a multi-option, and the system objects (status,
-- priority, resolution, issue type, project). For every other kind disjointness
-- is ordinary — a `scalar` or `datetime` "id" IS the value, so a field whose
-- values all moved on looks disjoint; attachment ids are per issue and never
-- repeat; sprint ids only ever grow.
--
-- COVERAGE. Only fields whose events reach a large enough share of the issues
-- holding a value. A field added recently has values and almost no history, and
-- that is not an id-space break — it is a young field.

{% set covered_kinds = ['option', 'option_array', 'obj'] %}

WITH scoped AS (
    SELECT insight_source_id, field_id, field_name, field_kind
    FROM {{ ref('jira__task_field_kind') }}
    WHERE field_kind IN ({{ "'" ~ covered_kinds | join("', '") ~ "'" }})
),

-- The ids the SOURCE named, so only changelog rows: a synthetic_initial row
-- carries a snapshot id, which would make every field intersect itself.
history_ids AS (
    SELECT insight_source_id, field_id, groupUniqArray(value_id) AS ids
    FROM (
        SELECT insight_source_id, field_id, arrayJoin(value_ids) AS value_id
        FROM {{ ref('jira__field_history_derived') }} FINAL
        WHERE event_kind = 'changelog'
    )
    WHERE value_id != ''
    GROUP BY insight_source_id, field_id
),

holders AS (
    SELECT DISTINCT insight_source_id, field_id, id_readable
    FROM (
        SELECT insight_source_id, field_id, id_readable, arrayJoin(value_ids) AS value_id
        FROM {{ ref('jira__issue_field_snapshot') }} FINAL
    )
    WHERE value_id != ''
),

current_ids AS (
    SELECT insight_source_id, field_id, groupUniqArray(value_id) AS ids
    FROM (
        SELECT insight_source_id, field_id, arrayJoin(value_ids) AS value_id
        FROM {{ ref('jira__issue_field_snapshot') }} FINAL
    )
    WHERE value_id != ''
    GROUP BY insight_source_id, field_id
),

changed AS (
    SELECT DISTINCT insight_source_id, field_id, id_readable
    FROM {{ ref('jira__changelog_items') }} FINAL
),

-- Coverage is the share of the issues HOLDING a value that also have an event
-- for it. Dividing two independent counts instead lets it exceed one — issues
-- with events but no current value are counted in the numerator only — and the
-- threshold then stops meaning anything.
coverage AS (
    SELECT
        h.insight_source_id                                  AS insight_source_id,
        h.field_id                                           AS field_id,
        count()                                              AS issues_holding_value,
        countIf(c.id_readable != '')                         AS issues_also_changed
    FROM holders AS h
    LEFT JOIN changed AS c
        ON c.insight_source_id = h.insight_source_id
       AND c.field_id = h.field_id
       AND c.id_readable = h.id_readable
    GROUP BY h.insight_source_id, h.field_id
)

SELECT
    s.insight_source_id                                   AS insight_source_id,
    s.field_id                                            AS field_id,
    s.field_name                                          AS field_name,
    s.field_kind                                          AS field_kind,
    length(h.ids)                                         AS ids_in_history,
    length(c.ids)                                         AS ids_in_current_values,
    v.issues_holding_value                                AS issues_holding_value,
    round(v.issues_also_changed / v.issues_holding_value, 2) AS event_coverage
FROM scoped AS s
INNER JOIN history_ids AS h
    ON h.insight_source_id = s.insight_source_id AND h.field_id = s.field_id
INNER JOIN current_ids AS c
    ON c.insight_source_id = s.insight_source_id AND c.field_id = s.field_id
INNER JOIN coverage AS v
    ON v.insight_source_id = s.insight_source_id AND v.field_id = s.field_id
WHERE length(arrayIntersect(h.ids, c.ids)) = 0
  AND v.issues_holding_value > 0
  AND v.issues_also_changed / v.issues_holding_value
      >= {{ var('jira_id_space_coverage_min', 0.5) }}
LIMIT 100
