{{ config(materialized='view', schema='staging', tags=['youtrack', 'staging']) }}

-- One row per issue: the latest snapshot from either Bronze stream. youtrack_issues
-- is the `updated:` search; youtrack_activity_issues re-reads the issues the
-- activity feed names, which covers changes that leave `updated` untouched.
-- The winner is chosen on keys alone so the payloads are never sorted; on a tie
-- the search snapshot wins.
{% set issue_streams = ['youtrack_issues', 'youtrack_activity_issues'] %}
{% set complete = 'id IS NOT NULL AND created IS NOT NULL AND source_id IS NOT NULL AND tenant_id IS NOT NULL' %}
WITH winners AS (
    SELECT tenant_id, source_id, id,
        argMax(stream, (_airbyte_extracted_at, stream_rank)) AS win_stream,
        max(_airbyte_extracted_at) AS win_at
    FROM (
    {%- for stream in issue_streams %}
        SELECT tenant_id, source_id, id, _airbyte_extracted_at,
            '{{ stream }}' AS stream, {{ loop.revindex0 }} AS stream_rank
        FROM {{ source('bronze_youtrack', stream) }} FINAL
        WHERE {{ complete }}
        {%- if not loop.last %}
        UNION ALL
        {%- endif %}
    {%- endfor %}
    )
    GROUP BY tenant_id, source_id, id
)
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
    WHERE {{ complete }} AND (tenant_id, source_id, id, _airbyte_extracted_at) IN (
        SELECT tenant_id, source_id, id, win_at FROM winners WHERE win_stream = '{{ stream }}')
    {%- if not loop.last %}
    UNION ALL
    {%- endif %}
{%- endfor %}
)
