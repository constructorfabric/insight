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
        {'field': 'workEmail',      'value_type': 'email',        'value_field_name': 'bronze_bamboohr.employees.workEmail'},
        {'field': 'employeeNumber', 'value_type': 'employee_id',  'value_field_name': 'bronze_bamboohr.employees.employeeNumber'},
        {'field': 'displayName',    'value_type': 'display_name', 'value_field_name': 'bronze_bamboohr.employees.displayName'},
    ],
    deactivation_condition="field_name = 'status' AND new_value IN ('Inactive', 'Terminated')"
) }}
