{{ config(
    severity='warn',
    tags=['connector_quality', 'jira'],
    store_failures=true,
    meta={
        'title': 'Same-instant changelog events of one field form a from-to chain',
        'domain': 'task-tracking',
        'category': 'consistency',
        'tier': 'warn',
        'remediation': 'Two or more changelog entries of one issue change the same field at the same millisecond, and no order of them lets each start where the previous one ended: a fork (two entries leaving one value) or a cycle across fields (one field says A came first, another says B did). The source does not record which happened first, so the journal falls back to the changelog id for those entries and the field\'s intermediate states there are a guess. Entries whose links contradict nothing keep their proven order. Nothing in the pipeline can repair this; a finding explains a round-trip failure on the same field.'
    }
) }}

-- SEVERITY: warn. The condition belongs to the source's changelog, which does
-- not say which of two same-instant entries came first; it counts where the
-- journal's order of such entries is the changelog id's guess rather than a
-- proven from→to chain.
--
-- Self-describing kinds only, and only the items the journal keeps: an
-- element-wise item names one element, not the field's value, so it states no
-- chain to break. Each entry is one event per field, collapsed as the journal
-- collapses it (`jira_entry_item_ends`); its several items are
-- assert_jira_entry_items_one_per_field's finding, not this one's.
--
-- Adjacent identical events are one step: whichever comes first, the field
-- passes through the same states.

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

entry_items AS (
    SELECT
        insight_source_id,
        issue_id,
        field_id,
        changelog_id,
        event_at,
        {{ jira_entry_items_ordered('groupArray((sides.1, sides.2, sides.3, sides.4))') }} AS items
    FROM live_items
    GROUP BY insight_source_id, issue_id, field_id, changelog_id, event_at
),

entry_events AS (
    SELECT
        insight_source_id,
        issue_id,
        field_id,
        changelog_id,
        event_at,
        items[ends.1].1                                   AS value_from,
        items[ends.2].3                                   AS value_to
    FROM (
        SELECT *, {{ jira_entry_item_ends('items') }} AS ends
        FROM entry_items
    )
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
        e.insight_source_id                               AS insight_source_id,
        e.issue_id                                        AS issue_id,
        e.field_id                                        AS field_id,
        e.event_at                                        AS event_at,
        arrayMap(x -> (x.2, x.3),
                 arraySort(x -> x.1, groupArray((j.event_order, e.value_from, e.value_to)))) AS ordered_steps,
        arrayFilter((s, k) -> k = 1 OR s != ordered_steps[k - 1],
                    ordered_steps, arrayEnumerate(ordered_steps)) AS steps
    FROM entry_events AS e
    INNER JOIN journal_order AS j
        ON j.insight_source_id = e.insight_source_id
       AND j.issue_id = e.issue_id
       AND j.field_id = e.field_id
       AND j.event_id = e.changelog_id
    GROUP BY e.insight_source_id, e.issue_id, e.field_id, e.event_at
    HAVING count() > 1
)

SELECT
    insight_source_id,
    issue_id,
    field_id,
    event_at,
    length(ordered_steps)                                 AS events
FROM same_instant_steps
WHERE arrayExists(k -> (steps[k]).2 != (steps[k + 1]).1, range(1, length(steps)))
LIMIT 100
