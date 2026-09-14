-- depends_on: {{ ref('github__task_links') }}
{{ config(
    materialized='incremental',
    incremental_strategy='delete+insert',
    unique_key='unique_key',
    schema='silver',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['silver']
) }}

-- Unified, source-neutral record of the links between work items, as
-- INTERVALS rather than as a current set: one row per link occurrence, open at
-- `valid_from`, closed at `valid_to`, still open when `valid_to` is NULL.
--
-- Reading the links that existed over a window is a range predicate:
--
--   WHERE valid_from < :to AND (valid_to IS NULL OR valid_to > :from)
--
-- and the links as of an instant is the same predicate with one point.
--
-- Both directions of a link are stored. A parent-child pair produces a
-- `sub_issue` row on the parent and a `parent` row on the child, because each
-- side is a fact about that side and collecting one issue must not depend on
-- having collected the other.
--
-- `evidence` distinguishes an interval folded from the vendor's own add and
-- remove events from one bounded by successive observations. A consumer that
-- mixes them without noticing is comparing a second-accurate edge with one
-- that is only known to a sync, so the column is not decoration.
--
-- `valid_from_known = 0` marks a lower bound rather than a fact: the link was
-- already there when collection began, or was only ever seen in a snapshot.
-- Filtering it out silently drops the oldest links, which are exactly the ones
-- a long window asks about.

SELECT * FROM (
    {{ union_by_tag('silver:class_task_links') }}
)
{% if is_incremental() %}
WHERE _version > (SELECT max(_version) FROM {{ this }})
{% endif %}
