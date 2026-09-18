{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['gitlab', 'silver', 'silver:identity_inputs']
) }}

-- Claims against GitLab accounts, unioned into silver.identity_inputs via the
-- `silver:identity_inputs` tag.
--
-- Two kinds of account, distinguished by source_type:
--
--   `gitlab`              — a real account, keyed on its immutable numeric
--                           GitLab user id, stringified (ADR-0002 binding key)
--                           — the same key gitlab__pull_requests puts on a
--                           merge request's author, so the two meet in the join.
--   `gitlab-commit-email` — an e-mail no account claims, keyed on the e-mail
--                           itself. Not a vendor account and never matched at
--                           sign-in: it exists so an operator can see the
--                           address in the console and merge it into the person
--                           it belongs to.
--
-- INVARIANT: every claim here is a fact GitLab states about the account
-- itself. A merge request's commits are NOT evidence about its author — the
-- author of a request and the author of a commit it carries are different
-- identities. The claim is append-only, so a wrong one could not be withdrawn.
--
-- Only `email` and `display_name` rows, never the `value_type='id'` binding
-- the shared macro also emits: what an account means is the persons-seed's
-- decision, not this model's.
--
-- Column order matches the macro's output — silver.identity_inputs is a
-- positional UNION ALL, and check-field-parity.py audits the shape.

WITH observations AS (
    SELECT
        toUUID(UUIDNumToString(sipHash128(coalesce(tenant_id, '')))) AS insight_tenant_id,
        toUUID(UUIDNumToString(sipHash128(coalesce(source_id, '')))) AS insight_source_id,
        'gitlab' AS insight_source_type,
        account_id AS source_account_id,
        'email' AS value_type,
        email AS value,
        observed_in AS value_field_name,
        'UPSERT' AS operation_type,
        -- Insertion time, NOT the historical observation time: the shared
        -- identity_inputs model admits only rows above its global max(_version)
        -- watermark, so a claim versioned by a past instant would be silently
        -- dropped whenever any other feeder has already stamped a newer
        -- version. The observation time survives on
        -- gitlab__account_emails.observed_at.
        now64(3) AS _synced_at
    FROM {{ ref('gitlab__account_emails') }}

    UNION ALL

    SELECT
        toUUID(UUIDNumToString(sipHash128(coalesce(tenant_id, '')))) AS insight_tenant_id,
        toUUID(UUIDNumToString(sipHash128(coalesce(source_id, '')))) AS insight_source_id,
        'gitlab' AS insight_source_type,
        account_id AS source_account_id,
        'display_name' AS value_type,
        name AS value,
        observed_in AS value_field_name,
        'UPSERT' AS operation_type,
        now64(3) AS _synced_at
    FROM {{ ref('gitlab__account_names') }}

    UNION ALL

    -- The unowned e-mail claims itself, so the console has an account to show
    -- and a value to search on.
    SELECT
        toUUID(UUIDNumToString(sipHash128(coalesce(tenant_id, '')))) AS insight_tenant_id,
        toUUID(UUIDNumToString(sipHash128(coalesce(source_id, '')))) AS insight_source_id,
        'gitlab-commit-email' AS insight_source_type,
        email AS source_account_id,
        'email' AS value_type,
        email AS value,
        'bronze_gitlab.commits.author_email' AS value_field_name,
        'UPSERT' AS operation_type,
        now64(3) AS _synced_at
    FROM {{ ref('gitlab__unowned_commit_emails') }}

    UNION ALL

    -- The git author name, so the operator recognises whose address it is
    -- rather than deciding on an e-mail alone.
    SELECT
        toUUID(UUIDNumToString(sipHash128(coalesce(tenant_id, '')))) AS insight_tenant_id,
        toUUID(UUIDNumToString(sipHash128(coalesce(source_id, '')))) AS insight_source_id,
        'gitlab-commit-email' AS insight_source_type,
        email AS source_account_id,
        'display_name' AS value_type,
        author_name AS value,
        'bronze_gitlab.commits.author_name' AS value_field_name,
        'UPSERT' AS operation_type,
        now64(3) AS _synced_at
    FROM {{ ref('gitlab__unowned_commit_emails') }}
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
    o.insight_tenant_id,
    o.insight_source_id,
    o.insight_source_type,
    o.source_account_id,
    o.value_type,
    o.value,
    o.value_field_name,
    o.operation_type,
    o._synced_at,
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
