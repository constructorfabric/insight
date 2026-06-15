-- depends_on: {{ ref('workday__bronze_promoted') }}
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['workday']
) }}

{# Last_Functionally_Updated is deliberately NOT tracked: it changes on any
   worker update, including fields we do not monitor, and would create
   spurious SCD2 versions. #}
{{ snapshot(
    source_ref=source('workday', 'workers'),
    unique_key_col='unique_key',
    check_cols=[
        'Display_Name', 'First_Name', 'Last_Name', 'Work_Email',
        'Business_Title', 'Job_Profile', 'Worker_Type', 'Worker_Status',
        'Supervisory_Organization',
        'Manager_Employee_ID', 'Manager_Work_Email',
        'Location', 'Country', 'City',
        'Hire_Date', 'Original_Hire_Date', 'Termination_Date',
        'Scheduled_Weekly_Hours'
    ],
    check_raw_data_cols=var('workday_custom_fields', [])
) }}
