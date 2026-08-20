-- depends_on: {{ ref('jira__bronze_promoted') }}
{{ config(
    materialized='table',
    schema='staging',
    tags=['jira', 'silver']
) }}

{{ fields_history(
    snapshot_ref=ref('jira__worklog_snapshot'),
    entity_id_col='worklog_id',
    fields=[
        'edited_at', 'is_deleted'
    ]
) }}
