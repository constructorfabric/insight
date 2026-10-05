{{ config(materialized='view', schema='staging', tags=['youtrack', 'staging']) }}

-- One row per issue: the latest snapshot from either Bronze stream. youtrack_issues
-- is the `updated:` search; youtrack_activity_issues re-reads the issues the
-- activity feed names, which covers changes that leave `updated` untouched.
{% set issue_streams = ['youtrack_issues', 'youtrack_activity_issues'] %}
SELECT assumeNotNull(tenant_id) AS tenant_id, assumeNotNull(source_id) AS insight_source_id,
    assumeNotNull(id) AS issue_id, coalesce(idReadable, '') AS id_readable,
    fromUnixTimestamp64Milli(toInt64(assumeNotNull(created))) AS created_at,
    reporter_id, coalesce(project_id, '') AS project_id,
    {{ youtrack_json('issue_json') }} AS payload,
    {{ youtrack_json('custom_fields_json') }} AS custom_fields,
    toDateTime64(_airbyte_extracted_at, 3) AS observed_at
FROM (
{%- for stream in issue_streams %}
    SELECT tenant_id, source_id, id, idReadable, created, reporter_id, project_id,
        issue_json, custom_fields_json, _airbyte_extracted_at
    FROM {{ source('bronze_youtrack', stream) }} FINAL
    WHERE id IS NOT NULL AND created IS NOT NULL AND source_id IS NOT NULL AND tenant_id IS NOT NULL
    {%- if not loop.last %}
    UNION ALL
    {%- endif %}
{%- endfor %}
)
ORDER BY _airbyte_extracted_at DESC
LIMIT 1 BY tenant_id, source_id, id
