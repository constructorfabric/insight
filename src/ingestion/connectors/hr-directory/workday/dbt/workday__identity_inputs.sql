{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['workday', 'silver', 'silver:identity_inputs']
) }}

{{ identity_inputs_from_history(
    fields_history_ref=ref('workday__workers_fields_history'),
    source_type='workday',
    identity_fields=[
        {'field': 'Work_Email',               'value_type': 'email',         'value_field_name': 'bronze_workday.workers.Work_Email'},
        {'field': 'Employee_ID',              'value_type': 'employee_id',   'value_field_name': 'bronze_workday.workers.Employee_ID'},
        {'field': 'Display_Name',             'value_type': 'display_name',  'value_field_name': 'bronze_workday.workers.Display_Name'},
        {'field': 'First_Name',               'value_type': 'first_name',    'value_field_name': 'bronze_workday.workers.First_Name'},
        {'field': 'Last_Name',                'value_type': 'last_name',     'value_field_name': 'bronze_workday.workers.Last_Name'},
        {'field': 'Supervisory_Organization', 'value_type': 'department',    'value_field_name': 'bronze_workday.workers.Supervisory_Organization'},
        {'field': 'Business_Title',           'value_type': 'job_title',     'value_field_name': 'bronze_workday.workers.Business_Title'},
        {'field': 'Worker_Status',            'value_type': 'status',        'value_field_name': 'bronze_workday.workers.Worker_Status'},
        {'field': 'Manager_Work_Email',       'value_type': 'parent_email',  'value_field_name': 'bronze_workday.workers.Manager_Work_Email'},
        {'field': 'Manager_Employee_ID',      'value_type': 'parent_id',     'value_field_name': 'bronze_workday.workers.Manager_Employee_ID'},
    ],
    deactivation_condition="field_name = 'Worker_Status' AND new_value = 'Terminated'"
) }}
