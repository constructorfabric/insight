{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by='(unique_key)',
    settings={'allow_nullable_key': 1},
    tags=['hubspot', 'silver:class_crm_activities']
) }}

{#- One arm per engagement object. Meetings and communications have no archived
    sibling: HubSpot answers HTTP 400 when paging their deleted records. -#}
{%- set null_string = 'CAST(NULL AS Nullable(String))' -%}
{%- set null_int = 'CAST(NULL AS Nullable(Int64))' -%}
{%- set null_time = 'CAST(NULL AS Nullable(DateTime64(3)))' -%}
{%- set arms = [
    {
        'table': 'engagements_calls', 'archived': true, 'type': 'call',
        'duration': 'intDiv(toInt64OrNull(properties_hs_call_duration), 1000)',
        'outcome': 'properties_hs_call_disposition',
        'metadata': {'title': 'properties_hs_call_title', 'direction': 'properties_hs_call_direction'},
        'subject': 'properties_hs_call_title', 'body': 'properties_hs_call_body',
        'direction': 'properties_hs_call_direction', 'status': 'properties_hs_call_status',
        'channel': null_string, 'ended_at': null_time,
    },
    {
        'table': 'engagements_emails', 'archived': true, 'type': 'email',
        'duration': null_int,
        'outcome': 'properties_hs_email_status',
        'metadata': {'subject': 'properties_hs_email_subject', 'direction': 'properties_hs_email_direction'},
        'subject': 'properties_hs_email_subject', 'body': 'properties_hs_email_text',
        'direction': 'properties_hs_email_direction', 'status': 'properties_hs_email_status',
        'channel': null_string, 'ended_at': null_time,
    },
    {
        'table': 'engagements_meetings', 'archived': false, 'type': 'meeting',
        'started_at': 'parseDateTime64BestEffortOrNull(toString(properties_hs_meeting_start_time), 3)',
        'duration': 'intDiv(toUnixTimestamp64Milli(parseDateTime64BestEffortOrNull(toString(properties_hs_meeting_end_time), 3)) - toUnixTimestamp64Milli(parseDateTime64BestEffortOrNull(toString(properties_hs_meeting_start_time), 3)), 1000)',
        'outcome': 'properties_hs_meeting_outcome',
        'metadata': {'title': 'properties_hs_meeting_title', 'location': 'properties_hs_meeting_location'},
        'subject': 'properties_hs_meeting_title', 'body': 'properties_hs_meeting_body',
        'direction': null_string, 'status': 'properties_hs_meeting_outcome',
        'channel': null_string,
        'ended_at': 'parseDateTime64BestEffortOrNull(toString(properties_hs_meeting_end_time), 3)',
    },
    {
        'table': 'engagements_tasks', 'archived': true, 'type': 'task',
        'duration': null_int,
        'outcome': 'properties_hs_task_status',
        'metadata': {'subject': 'properties_hs_task_subject', 'priority': 'properties_hs_task_priority', 'type': 'properties_hs_task_type'},
        'subject': 'properties_hs_task_subject', 'body': 'properties_hs_task_body',
        'direction': null_string, 'status': 'properties_hs_task_status',
        'channel': null_string, 'ended_at': null_time,
    },
    {
        'table': 'engagements_notes', 'archived': true, 'type': 'note',
        'duration': null_int,
        'outcome': null_string,
        'metadata': {},
        'subject': null_string, 'body': 'properties_hs_note_body',
        'direction': null_string, 'status': null_string,
        'channel': null_string, 'ended_at': null_time,
    },
    {
        'table': 'engagements_communications', 'archived': false, 'type': 'communication',
        'duration': null_int,
        'outcome': null_string,
        'metadata': {'channel': 'properties_hs_communication_channel_type', 'logged_from': 'properties_hs_communication_logged_from'},
        'subject': null_string, 'body': 'properties_hs_communication_body',
        'direction': null_string, 'status': null_string,
        'channel': 'properties_hs_communication_channel_type', 'ended_at': null_time,
    },
] -%}

WITH src AS (
    {% for arm in arms %}
    {% set arm_loop = loop %}
    {% for tbl in hubspot_bronze_tables(arm.table, archived=arm.archived) %}
    SELECT
        tenant_id,
        source_id,
        unique_key,
        id                                              AS activity_id,
        '{{ arm.type }}'                                AS activity_type,
        properties_hubspot_owner_id                     AS owner_id,
        -- Rep who logged the activity (owners.userId); the record owner above
        -- is often inherited from the contact owner.
        properties_hs_created_by_user_id                AS created_by_user_id,
        {{ hubspot_first_association('associations_contacts') }}  AS contact_id,
        {{ hubspot_first_association('associations_deals') }}     AS deal_id,
        {{ hubspot_first_association('associations_companies') }} AS account_id,
        -- Never NULL: silver keeps `timestamp` not_null.
        coalesce(
            {% if arm.started_at is defined %}{{ arm.started_at }},{% endif %}
            parseDateTime64BestEffortOrNull(toString(properties_hs_timestamp), 3),
            createdAt,
            toDateTime64(0, 3)
        )                                               AS timestamp,
        CAST({{ arm.duration }} AS Nullable(Int64))     AS duration_seconds,
        CAST({{ arm.outcome }} AS Nullable(String))     AS outcome,
        toJSONString(map(
            {%- for key, expr in arm.metadata.items() %}
            '{{ key }}', coalesce(toString({{ expr }}), ''),
            {%- endfor %}
            'archived', toString(coalesce(archived, false))
        ))                                              AS metadata,
        createdAt                                       AS created_at,
        data_source,
        {{ hubspot_record_version() }}                  AS _version,
        {{ hubspot_deletion_columns() }},
        {{ hubspot_properties_map() }}                  AS properties,
        CAST({{ arm.subject }} AS Nullable(String))     AS subject,
        CAST({{ arm.body }} AS Nullable(String))        AS body,
        CAST({{ arm.direction }} AS Nullable(String))   AS direction,
        CAST({{ arm.status }} AS Nullable(String))      AS status,
        CAST({{ arm.channel }} AS Nullable(String))     AS channel,
        CAST({{ arm.ended_at }} AS Nullable(DateTime64(3))) AS ended_at
    FROM {{ source('bronze_hubspot', tbl) }}
    {% if not (loop.last and arm_loop.last) %}UNION ALL{% endif %}
    {% endfor %}
    {% endfor %}
)

{{ hubspot_staging_tail('src') }}
