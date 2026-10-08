-- depends_on: {{ ref('jira__task_field_kind') }}
{{ config(
    materialized='table',
    alias='jira__changelog_entry_ranks',
    schema='staging',
    engine=insight_engine('ReplacingMergeTree'),
    order_by=['unique_key'],
    query_settings={
        'max_bytes_before_external_group_by': 2000000000,
    },
    tags=['staging', 'jira']
) }}

-- The rank of each changelog entry among the entries its issue recorded at the
-- same instant (FIELD-HISTORY-IN-DBT.md §5): one row per entry of such an
-- instant, none for an entry alone at its instant. `jira__field_history_derived`
-- reads it into `event_order`, and every field an entry changed shares it.
--
-- INVARIANT: its own table, not a CTE of the journal. The journal inlines every
-- CTE at each reference, and re-analysing the ordering lambdas at each one
-- stalls the query analyzer; a join to a table costs nothing to re-analyse.
--
-- Rebuilt whole on every run, like the changelog items it reads. MEMORY (§13):
-- only the narrow search for tied instants touches every item; kinds, sides and
-- arrays are computed for the tied instants alone.

WITH kinds AS (
    SELECT
        insight_source_id,
        field_id,
        field_kind
    FROM {{ ref('jira__task_field_kind') }}
    WHERE field_kind NOT IN ('ignored', 'UNKNOWN')
      AND field_id != 'created'
),

tied_instants AS (
    SELECT
        insight_source_id,
        assumeNotNull(jira_id)                            AS issue_id,
        created_at                                        AS event_at
    FROM {{ ref('jira__changelog_items') }}
    WHERE jira_id IS NOT NULL
    GROUP BY insight_source_id, issue_id, event_at
    HAVING min(toUInt64OrZero(changelog_id)) != max(toUInt64OrZero(changelog_id))
),

-- The journal's own live items (`jira_item_is_live`) of the tied instants, with
-- their sides resolved by the field's kind.
tied_items AS (
    SELECT
        ci.insight_source_id                              AS insight_source_id,
        assumeNotNull(ci.jira_id)                         AS issue_id,
        ci.created_at                                     AS event_at,
        ci.field_id                                       AS field_id,
        k.field_kind NOT IN {{ jira_element_wise_kinds() }}                AS self_describing,
        toUInt64OrZero(ci.changelog_id)                   AS event_ord,
        ci.changelog_id                                   AS changelog_id,
        {{ jira_delta_sides('k.field_kind', 'ci.value_from', 'ci.value_from_string',
                            'ci.value_to', 'ci.value_to_string') }}    AS sides
    FROM {{ ref('jira__changelog_items') }} AS ci
    INNER JOIN kinds AS k
        ON k.insight_source_id = ci.insight_source_id
       AND k.field_id = ci.field_id
    WHERE ci.jira_id IS NOT NULL
      AND (ci.insight_source_id, assumeNotNull(ci.jira_id), ci.created_at)
          IN (SELECT insight_source_id, issue_id, event_at FROM tied_instants)
      AND {{ jira_item_is_live('k.field_kind', 'ci.value_from', 'ci.value_from_string',
                               'ci.value_to', 'ci.value_to_string') }}
),

-- One event per (entry, field), collapsed exactly as the journal collapses it
-- (`jira_entry_item_ends`), so the chains below link the events it emits.
tied_entry_items AS (
    SELECT
        insight_source_id,
        issue_id,
        event_at,
        field_id,
        event_ord,
        changelog_id,
        any(self_describing)                              AS self_describing,
        {{ jira_entry_items_ordered('groupArray((sides.1, sides.2, sides.3, sides.4))') }} AS items
    FROM tied_items
    GROUP BY insight_source_id, issue_id, event_at, field_id, event_ord, changelog_id
),

tied_entry_events AS (
    SELECT
        insight_source_id,
        issue_id,
        event_at,
        field_id,
        event_ord,
        changelog_id,
        self_describing,
        items,
        {{ jira_entry_item_ends('items') }}               AS ends
    FROM tied_entry_items
),

tied_fields AS (
    SELECT
        insight_source_id,
        issue_id,
        event_at,
        field_id,
        any(self_describing)                              AS self_describing,
        arraySort(x -> (x.1, x.2),
                  groupUniqArray((event_ord, changelog_id, items[ends.1].1, items[ends.2].3))) AS evs,
        uniqExact(changelog_id)                           AS distinct_changelog_ids
    FROM tied_entry_events
    GROUP BY insight_source_id, issue_id, event_at, field_id
),

-- Element-wise items carry one element, not the field's value, so they link
-- nothing; their entries still take part in the instant's order.
tied_field_links AS (
    SELECT
        insight_source_id,
        issue_id,
        event_at,
        arrayMap(x -> (x.1, x.2), evs)                    AS entries,
        if(self_describing,
           {{ task_chain_walk('arrayMap(x -> x.3, evs)', 'arrayMap(x -> x.4, evs)',
                              'length(evs) > distinct_changelog_ids') }},
           CAST([] AS Array(UInt64)))                     AS walk,
        arrayMap(k -> ((evs[walk[k]]).2, (evs[walk[k + 1]]).2),
                 range(1, length(walk)))                  AS links
    FROM tied_fields
),

tied_links AS (
    SELECT
        insight_source_id,
        issue_id,
        arraySort(arrayDistinct(arrayFlatten(groupArray(entries))))  AS entries,
        arrayDistinct(arrayFlatten(groupArray(links)))    AS links
    FROM tied_field_links
    GROUP BY insight_source_id, issue_id, event_at
),

tied_orders AS (
    SELECT
        insight_source_id,
        issue_id,
        entries,
        {{ task_instant_order('length(entries)',
                              "arrayMap(l -> (toUInt64(indexOf(arrayMap(x -> x.2, entries), l.1)),
                                              toUInt64(indexOf(arrayMap(x -> x.2, entries), l.2))), links)") }}
                                                          AS entry_order
    FROM tied_links
),

entry_ranks AS (
    SELECT
        insight_source_id,
        issue_id,
        position.1                                        AS changelog_id,
        position.2                                        AS entry_rank
    FROM (
        -- INVARIANT: the arrayJoin stays alone in its own SELECT list. Reading
        -- its elements beside it re-evaluates it per reference.
        SELECT
            insight_source_id,
            issue_id,
            arrayJoin(arrayMap(k -> ((entries[entry_order[k]]).2, toUInt32(k - 1)),
                               range(1, length(entries) + 1)))  AS position
        FROM tied_orders
    )
)

SELECT
    CAST(concat(insight_source_id, '-', issue_id, '-', changelog_id) AS String) AS unique_key,
    insight_source_id,
    issue_id,
    changelog_id,
    entry_rank
FROM entry_ranks
