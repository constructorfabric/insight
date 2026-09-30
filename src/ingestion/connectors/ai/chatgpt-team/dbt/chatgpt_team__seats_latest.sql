{{ config(
    materialized='table',
    engine='ReplacingMergeTree',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['chatgpt-team']
) }}

-- The roster projection the identity chain reads. FINAL dedups the
-- ReplacingMergeTree bronze source before the snapshot compares versions (ADR-0001).

-- INVARIANT: user_id keys the whole identity chain, so a row without one is
-- dropped here rather than carried forward under an empty key. A row without
-- an email is kept — the account stays bindable by hand.

-- INVARIANT: every tracked column is '' rather than NULL. fields_history diffs
-- consecutive versions with `curr != prev`, and a comparison against NULL is
-- NULL rather than true, so a value that was cleared would produce no history
-- row at all and the identity chain would never learn it went away.

-- INVARIANT: a deactivated seat asserts no identity, and blanking the values
-- here is what makes that REVERSIBLE. The deletes themselves come from the
-- deactivation_condition in chatgpt_team__identity_inputs; this blanking exists
-- so the return publishes an UPSERT again. The macro re-emits an identity
-- observation only when the field's own value changes, so an account that came
-- back with an unchanged address would otherwise stay deleted forever.

WITH seat AS (

    SELECT
        tenant_id,
        source_id,
        unique_key,
        user_id,
        trim(coalesce(email, ''))               AS email,
        trim(coalesce(name, ''))                AS name,
        trim(coalesce(deactivated_time, ''))    AS deactivated_time
    FROM {{ source('bronze_chatgpt_team', 'chatgpt_team_seats') }} FINAL
    WHERE user_id IS NOT NULL
      AND trim(user_id) != ''

)

SELECT
    tenant_id,
    source_id,
    unique_key,
    user_id,
    if(deactivated_time != '', '', email)       AS email,
    if(deactivated_time != '', '', name)        AS name,
    deactivated_time
FROM seat
