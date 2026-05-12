-- depends_on: {{ ref('jira__bronze_promoted') }}
{{ config(
    materialized='table',
    schema='staging',
    tags=['jira', 'silver']
) }}

{{ fields_history(
    snapshot_ref=ref('jira__users_snapshot'),
    entity_id_col='account_id',
    fields=[
        'display_name', 'email', 'active', 'account_type'
    ]
) }}
