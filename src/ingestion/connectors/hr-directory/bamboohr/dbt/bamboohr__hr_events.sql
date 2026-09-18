{{ config(
    materialized='view',
    schema='staging',
    tags=['bamboohr', 'silver:class_hr_events']
) }}

SELECT
    lr.tenant_id                                            AS insight_tenant_id,
    lr.source_id,
    lr.unique_key,
    lr.employeeId                                           AS source_person_id,
    e.workEmail                                             AS email,
    'leave'                                                 AS event_type,
    JSONExtractString(toString(lr.type), 'name')            AS event_subtype,
    parseDateTimeBestEffortOrNull(lr.start)                 AS start_date,
    parseDateTimeBestEffortOrNull(lr.end)                   AS end_date,
    toFloat64OrNull(
        JSONExtractString(toString(lr.amount), 'amount')
    )                                                       AS duration_amount,
    JSONExtractString(toString(lr.amount), 'unit')          AS duration_unit,
    JSONExtractString(toString(lr.status), 'status')        AS request_status,
    'bamboohr'                                              AS source,
    parseDateTimeBestEffortOrNull(lr.created)               AS created_at,
    lr._airbyte_extracted_at                                AS ingested_at,
    toUnixTimestamp64Milli(lr._airbyte_extracted_at)        AS _version
FROM {{ source('bamboohr', 'leave_requests') }} lr
LEFT JOIN {{ source('bamboohr', 'employees') }} e
    ON lr.employeeId = e.id
    AND lr.tenant_id = e.tenant_id
WHERE lr.employeeId IS NOT NULL
  AND parseDateTimeBestEffortOrNull(lr.start) IS NOT NULL
  AND parseDateTimeBestEffortOrNull(lr.end)   IS NOT NULL
