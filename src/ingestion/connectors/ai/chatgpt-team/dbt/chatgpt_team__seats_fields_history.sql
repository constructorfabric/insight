{{ config(
    materialized='table',
    schema='staging',
    tags=['chatgpt-team', 'silver']
) }}

{{ fields_history(
    snapshot_ref=ref('chatgpt_team__seats_snapshot'),
    entity_id_col='user_id',
    fields=[
        'email', 'name'
    ]
) }}
