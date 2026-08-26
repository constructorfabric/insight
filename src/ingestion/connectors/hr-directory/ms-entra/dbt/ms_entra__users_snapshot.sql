{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['ms-entra']
) }}

{{ snapshot(
    source_ref=source('bronze_ms_entra', 'users'),
    unique_key_col='unique_key',
    check_cols=[
        'userPrincipalName', 'mail', 'displayName', 'givenName', 'surname',
        'employeeId', 'department', 'jobTitle', 'accountEnabled',
        'onPremisesSamAccountName', 'userType'
    ]
) }}
