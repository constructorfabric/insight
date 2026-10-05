{#
  The event stages of the field-history journal (`jira_journal_derive_stages`):
  the modelled, live changelog items of a batch, ranked within their instant,
  and the one event each entry contributes per self-describing field.
#}


{% macro jira_journal_ranked_events_sql() %}
WITH kinds AS (
    SELECT * FROM {{ jira_journal_stage('kinds') }}
),

changelog_items AS (
{{- jira_journal_changelog_items_sql() -}}
),

-- Every changelog item that belongs to a field we model, with its delta already
-- resolved by the field's kind.
events AS (
    SELECT
        ci.insight_source_id                              AS insight_source_id,
        ci.issue_id                                       AS issue_id,
        ci.changelog_id                                   AS changelog_id,
        -- INVARIANT: the changelog id orders two events of one instant only after
        -- the entry's rank, and only as a NUMBER — as text '101' sorts before
        -- '99'. The string form stays the event id, an identifier, not an order.
        toUInt64OrZero(ci.changelog_id)                   AS event_ord,
        ci.created_at                                     AS event_at,
        ci.author_account_id                              AS author_id,
        ci.field_id                                       AS field_id,
        k.field_name                                      AS field_name,
        k.field_kind                                      AS field_kind,
        {{ jira_delta_action('k.field_kind', 'ci.value_from', 'ci.value_from_string',
                             'ci.value_to', 'ci.value_to_string') }}   AS delta_action,
        {{ jira_delta_sides('k.field_kind', 'ci.value_from', 'ci.value_from_string',
                            'ci.value_to', 'ci.value_to_string') }}    AS sides,
        {{ jira_delta_element('ci.value_from', 'ci.value_from_string',
                              'ci.value_to', 'ci.value_to_string') }}  AS element,
        {{ jira_item_is_live('k.field_kind', 'ci.value_from', 'ci.value_from_string',
                             'ci.value_to', 'ci.value_to_string') }}   AS is_live,
        -- The item's identity within its entry: the four sides as written,
        -- which `jira__changelog_items` already keeps unique per entry.
        cityHash64(COALESCE(ci.value_from, ''), COALESCE(ci.value_from_string, ''),
                   COALESCE(ci.value_to, ''), COALESCE(ci.value_to_string, ''))  AS item_digest
    FROM changelog_items AS ci
    INNER JOIN kinds AS k
        ON k.insight_source_id = ci.insight_source_id
       AND k.field_id = ci.field_id
),

-- An item with nothing on either side carries no information (§6). Neither does
-- one whose two sides are spelled identically: Jira writes those as a
-- by-product of recalculating a field it did not change — a remaining estimate
-- re-stamped while an issue is closed is the common shape.
--
-- Dropping them is not merely tidiness. `initial_state` takes the `before` side
-- of the EARLIEST event, and items of one entry share (event_at, event_ord), so
-- a real change paired with a no-op left that choice to the planner: the same
-- data yielded either value from one run to the next.
--
-- Element-wise kinds are exempt, and must be: their sides carry ONE element,
-- absent on the side it is not on, so a no-op cannot arise — while an item
-- naming the same element on both sides is the RENAME of that element, whose
-- whole purpose is to carry the new display.
live_events AS (
    SELECT * FROM events
    WHERE is_live
),

-- ── the order of the entries that share an instant (§5) ────────────────────
-- An entry's rank among the issue's entries of its instant, shared by every
-- field it changed (`jira__changelog_entry_ranks`). An entry sharing its
-- instant with none has no row there and ranks 0.
ranked_events AS (
    SELECT
        e.*,
        COALESCE(r.entry_rank, toUInt32(0))               AS entry_rank
    FROM live_events AS e
    LEFT JOIN {{ ref('jira__changelog_entry_ranks') }} AS r FINAL
        ON r.insight_source_id = e.insight_source_id
       AND r.issue_id = e.issue_id
       AND r.changelog_id = e.changelog_id
)

SELECT * FROM ranked_events
{% endmacro %}


{% macro jira_journal_ordered_events_sql() %}
WITH live_events AS (
    SELECT * FROM {{ jira_journal_stage('ranked_events') }}
),

ranked_events AS (
    SELECT * FROM {{ jira_journal_stage('ranked_events') }}
),

-- ── one event per entry and field for the self-describing kinds (§5) ───────
-- INVARIANT: the items of one such field in one entry are one event, from the
-- head of their from→to chain to its tail, or one item standing for all of
-- them (`jira_entry_item_ends`). Kept apart they share every sort key and the
-- journal key, and leave the event to the planner.
-- MEMORY (§13): only this narrow aggregation touches every event; arrays are
-- gathered for the entries carrying several items alone.
entry_split_keys AS (
    SELECT
        insight_source_id,
        issue_id,
        field_id,
        changelog_id
    FROM live_events
    WHERE field_kind NOT IN {{ jira_element_wise_kinds() }}
    GROUP BY insight_source_id, issue_id, field_id, changelog_id
    HAVING count() > 1
),

entry_split_items AS (
    SELECT
        insight_source_id,
        issue_id,
        field_id,
        changelog_id,
        {{ jira_entry_items_ordered('groupArray((sides.1, sides.2, sides.3, sides.4, item_digest))') }} AS items
    FROM live_events
    WHERE field_kind NOT IN {{ jira_element_wise_kinds() }}
      AND (insight_source_id, issue_id, field_id, changelog_id)
          IN (SELECT insight_source_id, issue_id, field_id, changelog_id FROM entry_split_keys)
    GROUP BY insight_source_id, issue_id, field_id, changelog_id
),

entry_split_ends AS (
    SELECT
        insight_source_id,
        issue_id,
        field_id,
        changelog_id,
        items,
        {{ jira_entry_item_ends('items') }}               AS ends
    FROM entry_split_items
),

-- The item whose row the entry keeps, and the `from` side that row takes over.
-- INVARIANT: a scalar, not a CTE: evaluated once per query, while a CTE is
-- recomputed at every reference of `ordered_events`.
(SELECT groupArray((insight_source_id, issue_id, field_id, changelog_id,
                    items[ends.2].5, items[ends.1].1, items[ends.1].2))
 FROM entry_split_ends) AS entry_split_choices,

ordered_events AS (
    SELECT
        e.* REPLACE (if(COALESCE(c.collapsed, 0) = 1, (c.from_ids, c.from_displays, e.sides.3, e.sides.4), e.sides) AS sides)
    FROM ranked_events AS e
    LEFT JOIN (
        SELECT
            choice.1                                      AS insight_source_id,
            choice.2                                      AS issue_id,
            choice.3                                      AS field_id,
            choice.4                                      AS changelog_id,
            choice.5                                      AS kept_digest,
            choice.6                                      AS from_ids,
            choice.7                                      AS from_displays,
            toUInt8(1)                                    AS collapsed
        FROM (
            -- INVARIANT: the arrayJoin stays alone in its own SELECT list. Reading
            -- its elements beside it re-evaluates it per reference.
            SELECT arrayJoin(entry_split_choices) AS choice
        )
    ) AS c
        ON c.insight_source_id = e.insight_source_id
       AND c.issue_id = e.issue_id
       AND c.field_id = e.field_id
       AND c.changelog_id = e.changelog_id
    -- INVARIANT: COALESCE, not `c.collapsed = 0`: under join_use_nulls=1 an
    -- unmatched event reads NULL there, and the bare comparison drops it.
    WHERE e.field_kind NOT IN {{ jira_element_wise_kinds() }}
      AND (COALESCE(c.collapsed, 0) = 0 OR e.item_digest = c.kept_digest)
)

SELECT * FROM ordered_events
{% endmacro %}
