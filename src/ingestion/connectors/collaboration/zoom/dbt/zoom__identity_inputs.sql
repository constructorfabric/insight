{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['zoom', 'silver', 'silver:identity_inputs']
) }}

{{ identity_inputs_from_history(
    fields_history_ref=ref('zoom__users_fields_history'),
    source_type='zoom',
    identity_fields=[
        {'field': 'email',              'value_type': 'email',        'value_field_name': 'bronze_zoom.users.email'},
        {'field': 'employee_unique_id', 'value_type': 'employee_id',  'value_field_name': 'bronze_zoom.users.employee_unique_id'},
        {'field': 'display_name',       'value_type': 'display_name', 'value_field_name': 'bronze_zoom.users.display_name'},
    ],
    deactivation_condition="field_name = 'status' AND new_value = 'inactive'"
) }}
