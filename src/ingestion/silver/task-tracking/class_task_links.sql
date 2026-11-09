-- depends_on: {{ ref('youtrack__task_links') }}
-- depends_on: {{ ref('github__task_links') }}
-- depends_on: {{ ref('jira__task_links') }}
{{ config(
    materialized='incremental',
    on_schema_change='append_new_columns',
    pre_hook=[
        "{{ youtrack_reconcile_class() }}",
        "{{ reconcile_class_from_producer('jira') }}",
    ],
    incremental_strategy='delete+insert',
    unique_key='unique_key',
    schema='silver',
    engine=insight_engine('ReplacingMergeTree', '_version'),
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
-- remove events from one with an edge bounded by observations. A consumer that
-- mixes them without noticing is comparing a second-accurate edge with one
-- that is only known to a sync, so the column is not decoration.
--
-- `valid_from_known = 0` marks a lower bound rather than a fact: the link was
-- already there when collection began, or was only ever seen in a snapshot.
-- Filtering it out silently drops the oldest links, which are exactly the ones
-- a long window asks about.

SELECT candidate.* FROM (
    {{ union_by_tag('silver:class_task_links') }}
) AS candidate
-- YouTrack and Jira are reconciled whole by the pre_hooks, so their rows bypass
-- the boundary: a Jira row can change without new evidence when a linked key
-- first resolves to an id.
{{ silver_incremental_watermark(['tenant_id', 'insight_source_id', 'data_source'], always_reread="candidate.data_source IN ('youtrack', 'jira')") }}
