{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['gitlab', 'silver', 'silver:identity_inputs']
) }}

-- What GitLab itself reports about a user account, unioned into
-- silver.identity_inputs via the `silver:identity_inputs` tag.
--
-- The account id is the numeric GitLab user id, stringified — the same key
-- `gitlab__pull_requests` puts on a merge request's author (ADR-0002 binding
-- key), so the two meet in the join.
--
-- Two value types, from two different questions:
--   `email`        — which addresses the account itself publishes, the edge
--                    that lets an e-mail-keyed fact reach a person
--   `display_name` — who the account is, so an operator reviewing an unbound
--                    account can recognise it. GitLab returns `email` only to
--                    a token with admin scope and `public_email` only where the
--                    user filled it in, so for many accounts this is the only
--                    thing an operator has to go on.
--
-- INVARIANT: every claim here is a fact GitLab states about the account itself.
-- A merge request's commits are NOT evidence about its author — the author of a
-- request and the author of a commit it carries are different identities, and
-- `git.prs_merged` counts the request for the former. Inferring an address for
-- an account from a request's commits would hand one person's address to
-- another, and the claim is append-only, so it could not be withdrawn later.
--
-- No `value_type='id'` binding: what an account means is the persons-seed's
-- decision, not this model's. The seed attaches a claimed e-mail to whichever
-- person its roster binding or e-mail match names, and mints a new person for
-- an unmatched active account.
--
-- Column order matches the macro's output — silver.identity_inputs is a
-- positional UNION ALL, and check-field-parity.py audits the shape.

WITH observations AS (
    SELECT
        toUUID(UUIDNumToString(sipHash128(coalesce(tenant_id, '')))) AS insight_tenant_id,
        toUUID(UUIDNumToString(sipHash128(coalesce(source_id, '')))) AS insight_source_id,
        'gitlab' AS insight_source_type,
        toString(COALESCE(id, 0)) AS source_account_id,
        'email' AS value_type,
        lower(trimBoth(COALESCE(email, ''))) AS value,
        'bronze_gitlab.users.email' AS value_field_name,
        'UPSERT' AS operation_type,
        -- Insertion time, NOT the historical observation time: the shared
        -- identity_inputs model admits only rows above its global max(_version)
        -- watermark, so a claim versioned by a past instant would be silently
        -- dropped whenever any other feeder has already stamped a newer version.
        now64(3) AS _synced_at
    FROM {{ source('bronze_gitlab', 'users') }} FINAL
    WHERE COALESCE(id, 0) > 0
      AND COALESCE(email, '') != ''

    UNION ALL

    SELECT
        toUUID(UUIDNumToString(sipHash128(coalesce(tenant_id, '')))) AS insight_tenant_id,
        toUUID(UUIDNumToString(sipHash128(coalesce(source_id, '')))) AS insight_source_id,
        'gitlab' AS insight_source_type,
        toString(COALESCE(id, 0)) AS source_account_id,
        'email' AS value_type,
        lower(trimBoth(COALESCE(public_email, ''))) AS value,
        'bronze_gitlab.users.public_email' AS value_field_name,
        'UPSERT' AS operation_type,
        now64(3) AS _synced_at
    FROM {{ source('bronze_gitlab', 'users') }} FINAL
    WHERE COALESCE(id, 0) > 0
      AND COALESCE(public_email, '') != ''

    UNION ALL

    SELECT
        toUUID(UUIDNumToString(sipHash128(coalesce(tenant_id, '')))) AS insight_tenant_id,
        toUUID(UUIDNumToString(sipHash128(coalesce(source_id, '')))) AS insight_source_id,
        'gitlab' AS insight_source_type,
        toString(COALESCE(id, 0)) AS source_account_id,
        'display_name' AS value_type,
        COALESCE(name, '') AS value,
        'bronze_gitlab.users.name' AS value_field_name,
        'UPSERT' AS operation_type,
        now64(3) AS _synced_at
    FROM {{ source('bronze_gitlab', 'users') }} FINAL
    WHERE COALESCE(id, 0) > 0
      AND COALESCE(name, '') != ''
)

SELECT
    -- The value and the source are part of the key: two e-mails on one account
    -- stay two claims, and the same pair under two connections stays two
    -- accounts.
    CAST(concat(
        toString(o.insight_tenant_id), '-',
        toString(o.insight_source_id), '-',
        o.insight_source_type, '-',
        o.source_account_id, '-',
        o.value_type, '-',
        o.operation_type, '-',
        hex(sipHash64(o.value))
    ) AS String) AS unique_key,
    o.*,
    toUnixTimestamp64Milli(o._synced_at) AS _version
FROM observations AS o
{% if is_incremental() %}
LEFT ANTI JOIN {{ this }} AS existing
    ON  o.value_type                 = existing.value_type
    AND o.value                      = existing.value
    AND o.source_account_id          = existing.source_account_id
    AND existing.insight_source_type = o.insight_source_type
    AND existing.insight_tenant_id   = o.insight_tenant_id
    AND existing.insight_source_id   = o.insight_source_id
{% endif %}
