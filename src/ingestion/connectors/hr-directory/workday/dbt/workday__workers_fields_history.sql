-- depends_on: {{ ref('workday__bronze_promoted') }}
{{ config(
    materialized='table',
    schema='staging',
    tags=['workday', 'silver']
) }}

{{ fields_history(
    snapshot_ref=ref('workday__workers_snapshot'),
    entity_id_col='Employee_ID',
    fields=[
        'Display_Name', 'First_Name', 'Last_Name', 'Work_Email',
        'Business_Title', 'Job_Profile', 'Worker_Type', 'Worker_Status',
        'Supervisory_Organization',
        'Manager_Employee_ID', 'Manager_Work_Email',
        'Location', 'Country', 'City',
        'Hire_Date', 'Original_Hire_Date', 'Termination_Date',
        'Scheduled_Weekly_Hours'
    ],
    fields_raw_data=var('workday_custom_fields', [])
) }}
