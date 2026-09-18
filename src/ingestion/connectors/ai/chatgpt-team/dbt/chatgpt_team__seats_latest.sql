-- depends_on: {{ ref('chatgpt_team__bronze_promoted') }}
{{ config(
    materialized='table',
    engine='ReplacingMergeTree',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['chatgpt-team']
) }}

-- The roster projection the identity chain reads. FINAL dedups the promoted
-- ReplacingMergeTree source before the snapshot compares versions (ADR-0001).

-- INVARIANT: user_id keys the whole identity chain, so a row without one is
-- dropped here rather than carried forward under an empty key. A row without
-- an email is kept — the account stays bindable by hand.

SELECT
    tenant_id,
    source_id,
    unique_key,
    user_id,
    nullIf(trim(coalesce(email, '')), '')               AS email,
    nullIf(trim(coalesce(name, '')), '')                AS name
FROM {{ source('bronze_chatgpt_team', 'chatgpt_team_seats') }} FINAL
WHERE user_id IS NOT NULL
  AND trim(user_id) != ''
