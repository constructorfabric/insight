{{ config(
    materialized='table',
    schema='staging',
    tags=['active-directory', 'silver']
) }}

{{ fields_history(
    snapshot_ref=ref('active_directory__users_snapshot'),
    entity_id_col='id',
    fields=[
        'userPrincipalName', 'mail', 'displayName', 'givenName', 'surname',
        'employeeId', 'department', 'jobTitle', 'accountEnabled', 'status',
        'sAMAccountName'
    ]
) }}
