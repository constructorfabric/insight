{{ config(
    severity='warn',
    store_failures=true,
    meta={
        'title': 'M365 staging days were reported by the source',
        'domain': 'collab',
        'category': 'source_integrity',
        'tier': 'error',
        'remediation': 'problem=carried_forward_copy: a staging day whose bronze report holds a row with activity while its lastActivityDate is earlier is a copy Microsoft served for a day it never computed. The m365 staging feeders drop such days on build, so the rows predate the filter: a `dbt --full-refresh` over tag:m365+ (a MAJOR descriptor bump dispatches one) rebuilds staging, silver and gold without them. problem=unparseable_last_activity: the report sent lastActivityDate in a form whose first ten characters are not a date, so copies on that day cannot be detected and are kept; extend m365_last_activity_day for the new form.'
    }
) }}
{#- Relations are optional: an install that never synced M365 has neither the
    staging tables nor the bronze streams, and dbt compiles the test anyway. -#}
{%- set feeders = [
    ('email',      ref('m365__collab_email_activity'),               'email_activity'),
    ('chat',       ref('m365__collab_chat_activity'),                'teams_activity'),
    ('meeting',    ref('m365__collab_meeting_activity'),             'teams_activity'),
    ('onedrive',   ref('m365__collab_document_activity_onedrive'),   'onedrive_activity'),
    ('sharepoint', ref('m365__collab_document_activity_sharepoint'), 'sharepoint_activity'),
] -%}
{%- set present = [] -%}
{%- set streams = [] -%}
{%- for feeder, staging, stream in feeders -%}
    {%- if load_relation(staging) and load_relation(source('bronze_m365', stream)) -%}
        {%- do present.append((feeder, staging, stream)) -%}
        {%- if stream not in streams -%}
            {%- do streams.append(stream) -%}
        {%- endif -%}
    {%- endif -%}
{%- endfor -%}
{%- if present | length == 0 %}
SELECT '' AS problem, '' AS feeder, '' AS tenant_id, '' AS insight_source_id, toDate('1970-01-01') AS date
WHERE 0
{%- else %}
WITH staged AS (
    {%- for feeder, staging, stream in present %}
    SELECT DISTINCT
        '{{ feeder }}' AS feeder,
        '{{ stream }}' AS stream,
        ifNull(tenant_id, '') AS tenant_id,
        ifNull(insight_source_id, '') AS insight_source_id,
        date
    FROM {{ staging }}
    {%- if not loop.last %}
    UNION ALL
    {%- endif %}
    {%- endfor %}
),
reported AS (
    {%- for stream in streams %}
    SELECT
        '{{ stream }}' AS stream,
        tenant_id,
        insight_source_id,
        report_day AS date
    FROM ({{ m365_reported_days(stream) }})
    {%- if not loop.last %}
    UNION ALL
    {%- endif %}
    {%- endfor %}
)
SELECT 'carried_forward_copy' AS problem, feeder, tenant_id, insight_source_id, date
FROM staged
WHERE (stream, tenant_id, insight_source_id, date) NOT IN (
    SELECT stream, tenant_id, insight_source_id, date FROM reported
)
{%- for stream in streams %}

UNION ALL

SELECT DISTINCT
    'unparseable_last_activity' AS problem,
    '{{ stream }}' AS feeder,
    ifNull(tenant_id, '') AS tenant_id,
    ifNull(source_id, '') AS insight_source_id,
    toDate(reportRefreshDate) AS date
FROM {{ source('bronze_m365', stream) }} FINAL
WHERE ifNull(lastActivityDate, '') != ''
  AND {{ m365_last_activity_day() }} IS NULL
{%- endfor %}
{%- endif %}
