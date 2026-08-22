{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['bitbucket-cloud', 'silver', 'silver:identity_inputs']
) }}

-- E-mail claims against Bitbucket accounts, plus the roster that says which
-- accounts exist — unioned into silver.identity_inputs via the
-- `silver:identity_inputs` tag.
--
-- The account id is Bitbucket's `account_id`, the identifier the workspace
-- roster and every actor field resolve to (ADR-0002). `uuid` identifies the
-- same person but is not what the roster keys on.
--
-- Two kinds of account, distinguished by source_type:
--
--   `bitbucket`              — a real account, keyed on the account_id the
--                              workspace roster and every actor field resolve
--                              to (ADR-0002).
--   `bitbucket-commit-email` — an e-mail no account claims, keyed on the e-mail
--                              itself. Not a vendor account and never matched
--                              at sign-in: it exists so an operator can see the
--                              address in the console and merge it into the
--                              person it belongs to, which is the only way
--                              those commits ever attribute. A separate
--                              source_type keeps ADR-0002's account_id contract
--                              intact and keeps e-mail-shaped ids out of the
--                              (source_type, external_id) space the account
--                              lookup treats as unique.
--
-- Two value types, from two different questions:
--   `email`        — which addresses an account has committed under, the edge
--                    that lets an e-mail-keyed commit fact reach a person
--   `display_name` — who the account is, so an operator reviewing an unbound
--                    account can recognise it
--
-- No `value_type='id'` binding: what an account means is the persons-seed's
-- decision, not this model's. The seed attaches a claimed e-mail to whichever
-- person its roster binding or e-mail match names, and mints a new person for
-- an unmatched active account.
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
        'bitbucket' AS insight_source_type,
        account_id AS source_account_id,
        'email' AS value_type,
        email AS value,
        observed_in AS value_field_name,
        'UPSERT' AS operation_type,
        -- Insertion time, NOT the historical observation time: the shared
        -- identity_inputs model admits only rows above its global max(_version)
        -- watermark, so a claim versioned by a past commit date would be
        -- silently dropped whenever any other feeder has already stamped a
        -- newer version. The observation time survives on
        -- bitbucket_cloud__account_emails.observed_at.
        now64(3) AS _synced_at
    FROM {{ ref('bitbucket_cloud__account_emails') }}

    UNION ALL

    SELECT
        toUUID(UUIDNumToString(sipHash128(coalesce(tenant_id, '')))) AS insight_tenant_id,
        toUUID(UUIDNumToString(sipHash128(coalesce(source_id, '')))) AS insight_source_id,
        'bitbucket' AS insight_source_type,
        lower(trimBoth(COALESCE(account_id, ''))) AS source_account_id,
        'display_name' AS value_type,
        COALESCE(display_name, '') AS value,
        'bronze_bitbucket_cloud.workspace_members.display_name' AS value_field_name,
        'UPSERT' AS operation_type,
        now64(3) AS _synced_at
    FROM {{ source('bronze_bitbucket_cloud', 'workspace_members') }} FINAL
    WHERE COALESCE(account_id, '') != ''
      AND COALESCE(display_name, '') != ''

    UNION ALL

    -- The unowned e-mail claims itself, so the console has an account to show
    -- and a value to search on.
    SELECT
        toUUID(UUIDNumToString(sipHash128(coalesce(tenant_id, '')))) AS insight_tenant_id,
        toUUID(UUIDNumToString(sipHash128(coalesce(source_id, '')))) AS insight_source_id,
        'bitbucket-commit-email' AS insight_source_type,
        email AS source_account_id,
        'email' AS value_type,
        email AS value,
        'bronze_bitbucket_cloud.commits.author_email' AS value_field_name,
        'UPSERT' AS operation_type,
        now64(3) AS _synced_at
    FROM {{ ref('bitbucket_cloud__unowned_commit_emails') }}

    UNION ALL

    -- The git author name, so the operator recognises whose address it is
    -- rather than deciding on an e-mail alone. The persons-seed also names the
    -- person it mints from this.
    SELECT
        toUUID(UUIDNumToString(sipHash128(coalesce(tenant_id, '')))) AS insight_tenant_id,
        toUUID(UUIDNumToString(sipHash128(coalesce(source_id, '')))) AS insight_source_id,
        'bitbucket-commit-email' AS insight_source_type,
        email AS source_account_id,
        'display_name' AS value_type,
        author_name AS value,
        'bronze_bitbucket_cloud.commits.author_name' AS value_field_name,
        'UPSERT' AS operation_type,
        now64(3) AS _synced_at
    FROM {{ ref('bitbucket_cloud__unowned_commit_emails') }}
    WHERE author_name != ''
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
