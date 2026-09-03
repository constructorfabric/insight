{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['bamboohr', 'silver', 'silver:identity_inputs']
) }}

{{ identity_inputs_from_history(
    fields_history_ref=ref('bamboohr__employees_fields_history'),
    source_type='bamboohr',
    identity_fields=[
        {'field': 'workEmail',       'value_type': 'email',         'value_field_name': 'bronze_bamboohr.employees.workEmail'},
        {'field': 'employeeNumber',  'value_type': 'employee_id',   'value_field_name': 'bronze_bamboohr.employees.employeeNumber'},
        {'field': 'displayName',     'value_type': 'display_name',  'value_field_name': 'bronze_bamboohr.employees.displayName'},
        {'field': 'firstName',       'value_type': 'first_name',    'value_field_name': 'bronze_bamboohr.employees.firstName'},
        {'field': 'lastName',        'value_type': 'last_name',     'value_field_name': 'bronze_bamboohr.employees.lastName'},
        {'field': 'department',      'value_type': 'department',    'value_field_name': 'bronze_bamboohr.employees.department'},
        {'field': 'division',        'value_type': 'division',      'value_field_name': 'bronze_bamboohr.employees.division'},
        {'field': 'jobTitle',        'value_type': 'job_title',     'value_field_name': 'bronze_bamboohr.employees.jobTitle'},
        {'field': 'status',          'value_type': 'status',        'value_field_name': 'bronze_bamboohr.employees.status'},
        {'field': 'supervisorEmail', 'value_type': 'parent_email',  'value_field_name': 'bronze_bamboohr.employees.supervisorEmail'},
        {'field': 'supervisorEId',   'value_type': 'parent_id',     'value_field_name': 'bronze_bamboohr.employees.supervisorEId'},
    ],
    deactivation_condition="field_name = 'status' AND new_value IN ('Inactive', 'Terminated')",
    roster_membership={
        'kind': 'explicit_state',
        'active_when': "field_name = 'status' AND new_value = 'Active'",
        'inactive_when': "field_name = 'status' AND new_value IN ('Inactive', 'Terminated')",
    },
    person_profile_fields=[
        'email', 'display_name', 'first_name', 'last_name',
        'department', 'division', 'job_title', 'status'
    ]
) }}
