{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['zoom']
) }}

{{ snapshot(
    source_ref=source('bronze_zoom', 'users'),
    unique_key_col='unique_key',
    check_cols=[
        'first_name', 'last_name', 'display_name', 'email',
        'dept', 'status', 'role_id', 'timezone', 'language',
        'phone_number', 'employee_unique_id', 'type'
    ]
) }}
