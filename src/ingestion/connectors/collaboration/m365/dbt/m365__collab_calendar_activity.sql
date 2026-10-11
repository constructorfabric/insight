{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['m365', 'silver:class_collab_calendar_activity']
) }}

{%- set events = source('bronze_m365', 'calendar_events') %}
{%- set events_synced = adapter.get_relation(database=none, schema=events.schema, identifier='calendar_events') %}
WITH
bronze_events AS (
    {%- if events_synced %}
    SELECT
        tenant_id,
        source_id,
        userPrincipalName,
        id,
        _airbyte_extracted_at,
        start_time,
        end_time,
        response,
        other_invitees,
        showAs,
        isCancelled,
        isAllDay
    FROM {{ events }} FINAL
    {%- else %}
    -- WORKAROUND: the destination creates this table on the first sync with the stream.
    SELECT
        CAST(NULL AS Nullable(String)) AS tenant_id,
        CAST(NULL AS Nullable(String)) AS source_id,
        CAST(NULL AS Nullable(String)) AS userPrincipalName,
        CAST(NULL AS Nullable(String)) AS id,
        CAST(NULL AS DateTime64(3)) AS _airbyte_extracted_at,
        CAST(NULL AS Nullable(String)) AS start_time,
        CAST(NULL AS Nullable(String)) AS end_time,
        CAST(NULL AS Nullable(String)) AS response,
        CAST(NULL AS Nullable(Int64)) AS other_invitees,
        CAST(NULL AS Nullable(String)) AS showAs,
        CAST(NULL AS Nullable(Bool)) AS isCancelled,
        CAST(NULL AS Nullable(Bool)) AS isAllDay
    WHERE 0
    {%- endif %}
),

calendar_events AS (
    SELECT
        tenant_id,
        source_id,
        lower(userPrincipalName) AS person_key,
        userPrincipalName AS email,
        id AS event_id,
        _airbyte_extracted_at AS extracted_at,
        assumeNotNull(parseDateTimeBestEffortOrNull(start_time, 'UTC')) AS starts_at,
        assumeNotNull(parseDateTimeBestEffortOrNull(end_time, 'UTC')) AS ends_at,
        -- INVARIANT: a meeting is an accepted invitation, or one the person organized
        -- with at least one other person invited; tentative and unanswered do not count.
        ifNull(
            (
                response = 'accepted'
                OR (response = 'organizer' AND ifNull(other_invitees, 0) > 0)
            )
            AND showAs = 'busy'
            AND NOT ifNull(isCancelled, false)
            AND NOT ifNull(isAllDay, false)
            AND dateDiff('second', starts_at, ends_at) BETWEEN 1 AND 8 * 3600,
            false
        ) AS is_meeting
    FROM bronze_events
    WHERE userPrincipalName IS NOT NULL
      AND userPrincipalName != ''
      AND parseDateTimeBestEffortOrNull(start_time, 'UTC') IS NOT NULL
      AND parseDateTimeBestEffortOrNull(end_time, 'UTC') IS NOT NULL
),

event_days AS (
    SELECT
        tenant_id,
        source_id,
        person_key,
        email,
        event_id,
        extracted_at,
        is_meeting,
        event_day AS date,
        greatest(toInt64(toUnixTimestamp(starts_at)), toInt64(toUnixTimestamp(toDateTime(event_day, 'UTC')))) AS span_from,
        least(toInt64(toUnixTimestamp(ends_at)), toInt64(toUnixTimestamp(toDateTime(event_day, 'UTC') + INTERVAL 1 DAY))) AS span_until
    FROM calendar_events
    ARRAY JOIN arrayMap(
        day_offset -> addDays(toDate(starts_at, 'UTC'), day_offset),
        range(toUInt64(greatest(0, dateDiff('day', toDate(starts_at, 'UTC'), toDate(ends_at - INTERVAL 1 SECOND, 'UTC'))) + 1))
    ) AS event_day
    -- INVARIANT: only the 27 finished days the connector read are whole; an event that
    -- crosses either edge of that window must not leave a partial day behind.
    WHERE event_day >= toDate(extracted_at, 'UTC') - 27
      AND event_day < toDate(extracted_at, 'UTC')
),
{%- if is_incremental() %}

-- INVARIANT: the connector re-reads its whole window every sync, so every day of the
-- latest window is rebuilt, including days that no event covers any more.
dates_to_rebuild AS (
    SELECT arrayJoin(arrayMap(
        day_offset -> toDate(latest.extracted_at, 'UTC') - 27 + day_offset,
        range(27)
    )) AS date
    FROM (SELECT max(extracted_at) AS extracted_at FROM calendar_events) AS latest
    WHERE latest.extracted_at > toDateTime64(0, 3)
),
{%- endif %}

person_days AS (
    -- INVARIANT: overlapping meetings count once, so the spans are merged in start order.
    SELECT
        tenant_id,
        source_id,
        person_key,
        date,
        min(email) AS contact_email,
        toInt64(uniqExactIf(event_id, is_meeting)) AS meetings,
        toInt64(arrayFold(
            (covered, span) -> (
                covered.1 + greatest(0, span.2 - greatest(span.1, covered.2)),
                greatest(covered.2, span.2)
            ),
            arraySort(groupArrayIf((span_from, span_until), is_meeting)),
            (toInt64(0), toInt64(0))
        ).1) AS meeting_seconds
    FROM event_days
    {%- if is_incremental() %}
    WHERE date IN (SELECT date FROM dates_to_rebuild)
    {%- endif %}
    GROUP BY tenant_id, source_id, person_key, date
),
{%- if is_incremental() %}

vacated_person_days AS (
    -- INVARIANT: a rebuilt day with no events left for a person (renamed account, moved
    -- event) is written back as zero, so no stale hours outlive the rebuild.
    SELECT
        tenant_id,
        insight_source_id AS source_id,
        person_key,
        date,
        email AS contact_email,
        toInt64(0) AS meetings,
        toInt64(0) AS meeting_seconds
    FROM {{ this }}
    WHERE date IN (SELECT date FROM dates_to_rebuild)
      AND (tenant_id, insight_source_id, person_key, date) NOT IN (
          SELECT
              tenant_id,
              source_id,
              person_key,
              date
          FROM person_days
      )
),
{%- endif %}

rebuilt_person_days AS (
    SELECT
        tenant_id,
        source_id,
        person_key,
        date,
        contact_email,
        meetings,
        meeting_seconds
    FROM person_days
    {%- if is_incremental() %}
    UNION ALL
    SELECT
        tenant_id,
        source_id,
        person_key,
        date,
        contact_email,
        meetings,
        meeting_seconds
    FROM vacated_person_days
    {%- endif %}
)

SELECT
    tenant_id,
    source_id AS insight_source_id,
    MD5(concat(tenant_id, '-', source_id, '-', person_key, '-', toString(date))) AS unique_key,
    contact_email AS email,
    person_key,
    date,
    meetings,
    meeting_seconds,
    now() AS collected_at,
    'insight_m365' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version
FROM rebuilt_person_days
