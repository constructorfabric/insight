{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['github', 'silver', 'silver:identity_inputs']
) }}

-- E-mail claims against GitHub accounts, unioned into silver.identity_inputs
-- via the `silver:identity_inputs` tag.
--
-- source_type is 'github', matching the roster connector: both describe the
-- same accounts, so both must name the same vendor. The account id is the
-- lowercased login (ADR-0002).
--
-- Only `value_type='email'` rows, never the `value_type='id'` binding the
-- shared macro also emits: what an account means is the persons-seed's decision,
-- not this model's. The seed attaches a claimed e-mail to whichever person its
-- roster binding or e-mail match names — which is how an organization rostered
-- from an HR source and using only git still gets commit attribution — and mints
-- a new person for an unmatched active account.
--
-- Hand-rolled rather than built with identity_inputs_from_history: that macro
-- keys a row on (account, value_type, instant) without the value, which holds
-- for a roster carrying one e-mail per member but not here, where an account
-- legitimately has several e-mails at once and all of them must survive.
--
-- Column order matches the macro's output — silver.identity_inputs is a
-- positional UNION ALL, and check-field-parity.py audits the shape.

WITH observations AS (
    SELECT
        toUUID(UUIDNumToString(sipHash128(coalesce(tenant_id, '')))) AS insight_tenant_id,
        toUUID(UUIDNumToString(sipHash128(coalesce(source_id, '')))) AS insight_source_id,
        'github' AS insight_source_type,
        login AS source_account_id,
        'email' AS value_type,
        email AS value,
        observed_in AS value_field_name,
        'UPSERT' AS operation_type,
        -- Insertion time, NOT the historical observation time: the shared
        -- identity_inputs model admits only rows above its global max(_version)
        -- watermark, so a claim versioned by a past commit date would be
        -- silently dropped whenever any other feeder has already stamped a
        -- newer version. The observation time survives on
        -- github__account_emails.observed_at.
        now64(3) AS _synced_at
    FROM {{ ref('github__account_emails') }}
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
    AND existing.insight_source_type = 'github'
    AND existing.insight_tenant_id   = o.insight_tenant_id
    AND existing.insight_source_id   = o.insight_source_id
{% endif %}
