{{ config(
    materialized='view',
    schema='staging',
    tags=['bamboohr', 'silver:class_person_absences']
) }}

WITH snapshots AS (
    SELECT
        tenant_id,
        source_id,
        argMax(
            tuple(entries_json, window_start, window_end),
            tuple(_airbyte_extracted_at, _airbyte_raw_id)
        ) AS snapshot
    FROM {{ source('bamboohr', 'whos_out') }}
    WHERE entries_json IS NOT NULL
    GROUP BY tenant_id, source_id
),

entries AS (
    SELECT
        coalesce(tenant_id, '') AS insight_tenant_id,
        coalesce(source_id, '') AS account_source_id,
        toDateOrNull(snapshot.2) AS window_start,
        toDateOrNull(snapshot.3) AS window_end,
        arrayJoin(JSONExtractArrayRaw(assumeNotNull(snapshot.1))) AS entry
    FROM snapshots
),

intervals AS (
    SELECT
        insight_tenant_id,
        account_source_id,
        JSON_VALUE(entry, '$.employeeId') AS account_id,
        greatest(toDateOrNull(JSONExtractString(entry, 'start')), window_start) AS start_date,
        least(toDateOrNull(JSONExtractString(entry, 'end')), window_end) AS end_date
    FROM entries
    WHERE JSONExtractString(entry, 'type') = 'timeOff'
      AND JSONType(entry, 'employeeId') IN ('String', 'Int64', 'UInt64')
      AND toDateOrNull(JSONExtractString(entry, 'start')) IS NOT NULL
      AND toDateOrNull(JSONExtractString(entry, 'end')) IS NOT NULL
      AND window_start IS NOT NULL
      AND window_end IS NOT NULL
)

SELECT DISTINCT
    insight_tenant_id,
    toJSONString(tuple(insight_tenant_id, account_source_id, account_id, start_date, end_date)) AS unique_key,
    'bamboohr' AS account_source_type,
    account_source_id,
    account_id,
    assumeNotNull(start_date) AS start_date,
    assumeNotNull(end_date) AS end_date
FROM intervals
WHERE account_id != ''
  AND start_date <= end_date
