-- depends_on: {{ ref('zendesk__support_activity') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    incremental_strategy='delete+insert',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='silver',
    tags=['silver']
) }}

-- Unified person × date support activity across all support vendors
-- (Zendesk today; Freshdesk/etc. plug in by emitting a per-source staging
-- model tagged `silver:class_support_activity`). Same grain and union
-- mechanism as silver/collaboration/class_collab_*_activity — so a person's
-- support activity can be compared on equal footing with their wiki / chat /
-- meeting / code activity in Gold.
--
-- Column families (avoid double-counting when aggregating in Gold):
--   • Activity counts (actor-attributed): updates, public_comments,
--     private_comments, solved — each independent, sum freely. Honest NULL
--     for a vendor/period not yet wired to its audit stream.
--   • Authoring: kb_articles_created.
--   • Quality (assignee-attributed, NOT activity): csat_good / csat_total —
--     a ratio metric (Σgood / Σtotal), do not add to the activity counts.
SELECT * FROM (
    {{ union_by_tag('silver:class_support_activity') }}
)
{% if is_incremental() %}
WHERE _version > (SELECT max(_version) FROM {{ this }})
{% endif %}
