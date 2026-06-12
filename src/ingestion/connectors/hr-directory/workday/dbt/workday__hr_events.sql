-- depends_on: {{ ref('workday__bronze_promoted') }}
{{ config(
    materialized='view',
    schema='staging',
    tags=['workday', 'silver:class_hr_events']
) }}

SELECT
    lr.tenant_id                                            AS insight_tenant_id,
    lr.source_id,
    lr.unique_key,
    lr.Employee_ID                                          AS source_person_id,
    w.Work_Email                                            AS email,
    'leave'                                                 AS event_type,
    lr.Time_Off_Type                                        AS event_subtype,
    parseDateTimeBestEffortOrNull(lr.Start_Date)            AS start_date,
    parseDateTimeBestEffortOrNull(lr.End_Date)              AS end_date,
    toFloat64OrNull(toString(lr.Quantity))                  AS duration_amount,
    lr.Unit                                                 AS duration_unit,
    lr.Status                                               AS request_status,
    'workday'                                               AS source,
    parseDateTimeBestEffortOrNull(lr.Submitted_Moment)      AS created_at,
    lr._airbyte_extracted_at                                AS ingested_at,
    toUnixTimestamp64Milli(lr._airbyte_extracted_at)        AS _version
FROM {{ source('workday', 'leave_requests') }} lr
LEFT JOIN {{ source('workday', 'workers') }} w
    ON lr.Employee_ID = w.Employee_ID
    AND lr.tenant_id = w.tenant_id
    AND lr.source_id = w.source_id
WHERE lr.Employee_ID IS NOT NULL
  AND parseDateTimeBestEffortOrNull(lr.Start_Date) IS NOT NULL
  AND parseDateTimeBestEffortOrNull(lr.End_Date)   IS NOT NULL
