{{ config(
    materialized='table',
    schema='staging',
    tags=['ms-entra', 'silver']
) }}

{{ fields_history(
    snapshot_ref=ref('ms_entra__users_snapshot'),
    entity_id_col='id',
    fields=[
        'userPrincipalName', 'mail', 'displayName', 'givenName', 'surname',
        'employeeId', 'department', 'jobTitle', 'accountEnabled',
        'onPremisesSamAccountName', 'userType'
    ]
) }}
