-- depends_on: {{ ref('jira__bronze_promoted') }}
{{ config(
    materialized='table',
    schema='staging',
    tags=['jira', 'silver']
) }}

{{ fields_history(
    snapshot_ref=ref('jira__issue_availability_snapshot'),
    entity_id_col='jira_id',
    fields=[
        'availability'
    ]
) }}
