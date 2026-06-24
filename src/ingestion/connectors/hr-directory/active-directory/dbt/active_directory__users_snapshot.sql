-- depends_on: {{ ref('active_directory__bronze_promoted') }}
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['active-directory']
) }}

{{ snapshot(
    source_ref=source('bronze_active_directory', 'users'),
    unique_key_col='unique_key',
    check_cols=[
        'userPrincipalName', 'mail', 'displayName', 'givenName', 'surname',
        'employeeId', 'department', 'jobTitle', 'accountEnabled', 'status',
        'sAMAccountName'
    ]
) }}
