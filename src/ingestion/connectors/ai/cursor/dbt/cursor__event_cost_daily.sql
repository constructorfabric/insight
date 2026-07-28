-- Cursor priced usage rolled up from the per-event grain to (tenant, source,
-- email, day). Single definition of what a day's Cursor cost is, shared by
-- cursor__ai_dev_usage (which lands it on the person-day) and
-- assert_cursor_event_cost_attributed (which checks none of it is lost).
--
-- `chargedCents` is an event's full priced value and ALREADY INCLUDES
-- `cursorTokenFee` — summing both double-counts the fee. Every event kind is
-- included: `Included in Business` events are priced usage a seat covered, so
-- this total is usage at Cursor's rates, not an invoiced amount.
--
-- Ephemeral: inlined as a CTE by each consumer, so it adds no relation and each
-- consumer keeps its own day bound.
{{ config(
    materialized='ephemeral',
    tags=['cursor']
) }}

SELECT
    coalesce(tenant_id, '')                                     AS tenant_key,
    coalesce(source_id, '')                                     AS source_key,
    lower(trim(userEmail))                                      AS email,
    toDate(fromUnixTimestamp64Milli(toInt64OrNull(timestamp)))   AS day,
    toInt64(round(sum(coalesce(chargedCents, 0))))               AS charged_cents,
    max(toUnixTimestamp64Milli(_airbyte_extracted_at))           AS cost_version,
    toUInt8(count(chargedCents) > 0)                             AS has_priced_event
FROM {{ ref('cursor__usage_events') }}
WHERE userEmail IS NOT NULL
  AND trim(userEmail) != ''
  AND toInt64OrNull(timestamp) IS NOT NULL
GROUP BY tenant_key, source_key, email, day
