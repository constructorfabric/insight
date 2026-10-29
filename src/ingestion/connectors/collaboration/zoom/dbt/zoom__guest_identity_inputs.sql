{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['zoom', 'silver', 'silver:identity_inputs']
) }}

-- Guest accounts for the identity console, unioned into silver.identity_inputs
-- via the `silver:identity_inputs` tag.
--
-- A Zoom participant who joined without signing in carries only the display
-- name typed into the client. When no Zoom user of the account is named after
-- it, `zoom__collab_meeting_activity` keeps the sessions under a `zoom-guest`
-- account keyed on the normalized name, and this model states that account so
-- an operator can see it in the console and bind it to the person it belongs
-- to — which is the only way those meetings ever attribute. A separate
-- source_type keeps the name-shaped ids out of the space `zoom` keys on user
-- ids, as `bitbucket-commit-email` does for addresses.
--
-- Only `display_name` is stated: the account is a claim, not a participant,
-- so no `id` row — the persons-seed never mints a person for it and the
-- binding an operator writes is the only `id` it ever carries.
--
-- INVARIANT: a name seen on one day only is not stated: a one-off name is
-- more likely a visitor's than an employee's, and listing every one would bury
-- the names worth binding. The sessions themselves are kept, so the day the
-- name recurs it appears here and a binding reaches that first day too.

WITH observations AS (
    SELECT
        toUUID(UUIDNumToString(sipHash128(coalesce(tenant_id, '')))) AS insight_tenant_id,
        toUUID(UUIDNumToString(sipHash128(coalesce(insight_source_id, '')))) AS insight_source_id,
        'zoom-guest' AS insight_source_type,
        account_id AS source_account_id,
        'display_name' AS value_type,
        argMax(coalesce(user_name, ''), date) AS value,
        'bronze_zoom.participants.user_name' AS value_field_name,
        'UPSERT' AS operation_type,
        -- Insertion time, not the meeting date: the shared identity_inputs
        -- model admits only rows above its watermark.
        now64(3) AS _synced_at
    FROM {{ ref('zoom__collab_meeting_activity') }} FINAL
    WHERE account_source_type = 'zoom-guest'
      AND account_id != ''
    GROUP BY tenant_id, insight_source_id, account_id
    HAVING uniqExact(date) >= 2
       AND value != ''
)

SELECT
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
