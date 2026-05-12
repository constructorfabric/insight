-- depends_on: {{ ref('bamboohr__bronze_promoted') }}
{{ config(
    materialized='table',
    schema='staging',
    tags=['bamboohr', 'silver']
) }}

{{ fields_history(
    snapshot_ref=ref('bamboohr__employees_snapshot'),
    entity_id_col='id',
    fields=[
        'displayName', 'firstName', 'lastName', 'workEmail',
        'employeeNumber', 'jobTitle', 'department', 'division',
        'status', 'employmentHistoryStatus',
        'supervisorEId', 'supervisorEmail',
        'location', 'country', 'city',
        'hireDate', 'terminationDate'
    ],
    fields_raw_data=var('bamboohr_custom_fields', [])
) }}
