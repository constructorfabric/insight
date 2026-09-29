{{ config(
    severity='warn',
    tags=['connector_quality', 'jira'],
    store_failures=true,
    meta={
        'title': 'Same-instant changelog events of one field form a from-to chain',
        'domain': 'task-tracking',
        'category': 'consistency',
        'tier': 'warn',
        'remediation': 'Two or more changelog entries of one issue change the same field at the same millisecond, and no order of them lets each start where the previous one ended: a fork (two entries leaving one value), a cycle across fields (one field says A came first, another says B did), or one entry carrying two items of the field. The source does not record which happened first, so the journal falls back to the changelog id for those entries and the field\'s intermediate states there are a guess. Entries whose links contradict nothing keep their proven order. Nothing in the pipeline can repair this; a finding explains a round-trip failure on the same field.'
    }
) }}

-- SEVERITY: warn. The condition belongs to the source's changelog, which does
-- not say which of two same-instant entries came first; it counts where the
-- journal's order of such entries is the changelog id's guess rather than a
-- proven from→to chain.
--
-- Self-describing kinds only, and only the items the journal keeps: an
-- element-wise item names one element, not the field's value, so it states no
-- chain to break.

WITH kinds AS (
    SELECT
        insight_source_id,
        field_id,
        field_kind
    FROM {{ ref('jira__task_field_kind') }}
    WHERE field_kind NOT IN ('ignored', 'UNKNOWN')
      AND field_kind NOT IN {{ jira_element_wise_kinds() }}
),

live_items AS (
    SELECT
        ci.insight_source_id                              AS insight_source_id,
        assumeNotNull(ci.jira_id)                         AS issue_id,
        ci.field_id                                       AS field_id,
        ci.changelog_id                                   AS changelog_id,
        ci.created_at                                     AS event_at,
        {{ jira_delta_sides('k.field_kind', 'ci.value_from', 'ci.value_from_string',
                            'ci.value_to', 'ci.value_to_string') }}    AS sides
    FROM {{ ref('jira__changelog_items') }} AS ci
    INNER JOIN kinds AS k
        ON k.insight_source_id = ci.insight_source_id
       AND k.field_id = ci.field_id
    WHERE ci.jira_id IS NOT NULL
      AND {{ jira_item_is_live('k.field_kind', 'ci.value_from', 'ci.value_from_string',
                               'ci.value_to', 'ci.value_to_string') }}
),

journal_order AS (
    SELECT
        insight_source_id,
        issue_id,
        field_id,
        event_id,
        event_order
    FROM {{ ref('jira__field_history_derived') }} FINAL
    WHERE event_kind = 'changelog'
),

same_instant_steps AS (
    SELECT
        i.insight_source_id                               AS insight_source_id,
        i.issue_id                                        AS issue_id,
        i.field_id                                        AS field_id,
        i.event_at                                        AS event_at,
        arrayMap(x -> (x.2, x.3),
                 arraySort(x -> x.1, groupArray((j.event_order, i.sides.1, i.sides.3)))) AS steps
    FROM live_items AS i
    INNER JOIN journal_order AS j
        ON j.insight_source_id = i.insight_source_id
       AND j.issue_id = i.issue_id
       AND j.field_id = i.field_id
       AND j.event_id = i.changelog_id
    GROUP BY i.insight_source_id, i.issue_id, i.field_id, i.event_at
    HAVING count() > 1
)

SELECT
    insight_source_id,
    issue_id,
    field_id,
    event_at,
    length(steps)                                         AS events
FROM same_instant_steps
WHERE arrayExists(k -> (steps[k]).2 != (steps[k + 1]).1, range(1, length(steps)))
LIMIT 100
