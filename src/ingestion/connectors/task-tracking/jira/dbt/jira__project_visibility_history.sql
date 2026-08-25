-- depends_on: {{ ref('jira__bronze_promoted') }}
{{ config(
    materialized='table',
    schema='staging',
    tags=['jira', 'silver']
) }}

{{ fields_history(
    snapshot_ref=ref('jira__project_visibility_snapshot'),
    entity_id_col='project_id',
    fields=[
        'is_visible', 'project_status'
    ]
) }}
