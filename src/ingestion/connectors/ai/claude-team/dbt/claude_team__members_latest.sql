{{ config(
    materialized='table',
    engine='ReplacingMergeTree',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['claude-team']
) }}

-- Flattens the serialized `account` object of bronze_claude_team.claude_team_members.
-- The snapshot macro hashes named columns and fields_history tracks them by
-- name, so neither can reach a field nested inside it; the identity chain needs
-- them as ordinary columns. FINAL dedups the promoted ReplacingMergeTree source
-- before the snapshot compares versions (ADR-0001).

-- WORKAROUND: the destination represents `account` as String or as JSON
-- depending on the installation; subfield access compiles only against the
-- latter, while `toString` yields the same serialized text for both.

-- INVARIANT: account_uuid keys the whole identity chain, so a row without one is
-- dropped here rather than carried forward under an empty key. A row without an
-- email is kept — the account stays bindable by hand.

SELECT
    tenant_id,
    source_id,
    unique_key,
    JSONExtractString(COALESCE(toString(account), '{}'), 'uuid')          AS account_uuid,
    JSONExtractString(COALESCE(toString(account), '{}'), 'email_address') AS email_address,
    JSONExtractString(COALESCE(toString(account), '{}'), 'full_name')     AS full_name
FROM {{ source('bronze_claude_team', 'claude_team_members') }} FINAL
WHERE account_uuid != ''
