{# WORKAROUND: Microsoft Graph answers a usage-report day it never computed with a
   copy of the last computed report, stamped with the requested date. A real report
   cannot hold a row with activity on the day while its lastActivityDate is earlier;
   a copy holds such rows for every active user. A quiet day, or a day without
   lastActivityDate at all, has none and is kept. #}

{#- The calendar date prefix, so a timestamp form keeps its day without a
    timezone shift; anything else is NULL and kept, never read as a copy. -#}
{% macro m365_last_activity_day() -%}
toDateOrNull(left(lastActivityDate, 10))
{%- endmacro %}

{% macro m365_activity_expr(stream) %}
{%- set columns = {
    'email_activity': ['sendCount', 'readCount'],
    'teams_activity': [
        'teamChatMessageCount', 'privateChatMessageCount', 'postMessages', 'replyMessages',
        'callCount', 'meetingsAttendedCount', 'meetingsOrganizedCount'
    ],
    'onedrive_activity': [
        'viewedOrEditedFileCount', 'syncedFileCount',
        'sharedInternallyFileCount', 'sharedExternallyFileCount'
    ],
    'sharepoint_activity': [
        'viewedOrEditedFileCount', 'syncedFileCount',
        'sharedInternallyFileCount', 'sharedExternallyFileCount', 'visitedPageCount'
    ],
} -%}
{%- if stream not in columns -%}
    {{ exceptions.raise_compiler_error("m365_activity_expr: unknown stream '" ~ stream ~ "'") }}
{%- endif -%}
{%- for column in columns[stream] -%}
ifNull({{ column }}, 0){% if not loop.last %} + {% endif %}
{%- endfor %} > 0
{%- endmacro %}

{% macro m365_reported_days(stream) %}
SELECT
    ifNull(tenant_id, '') AS tenant_id,
    ifNull(source_id, '') AS insight_source_id,
    toDate(reportRefreshDate) AS report_day
FROM {{ source('bronze_m365', stream) }} FINAL
GROUP BY tenant_id, insight_source_id, report_day
HAVING countIf(
    {{ m365_last_activity_day() }} < report_day
    AND {{ m365_activity_expr(stream) }}
) = 0
{%- endmacro %}

{% macro m365_day_was_reported(stream) %}
(ifNull(tenant_id, ''), ifNull(source_id, ''), toDate(reportRefreshDate)) IN (
    {{ m365_reported_days(stream) }}
)
{%- endmacro %}
