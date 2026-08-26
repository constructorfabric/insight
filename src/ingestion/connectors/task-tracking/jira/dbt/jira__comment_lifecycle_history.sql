-- depends_on: {{ ref('jira__bronze_promoted') }}
{{ config(
    materialized='table',
    schema='staging',
    tags=['jira', 'silver']
) }}

{{ fields_history(
    snapshot_ref=ref('jira__comment_snapshot'),
    entity_id_col='comment_id',
    fields=[
        'edited_at', 'is_deleted'
    ]
) }}
