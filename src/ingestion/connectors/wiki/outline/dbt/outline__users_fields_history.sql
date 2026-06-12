{{ config(
    materialized='table',
    schema='staging',
    tags=['outline', 'silver']
) }}

{{ fields_history(
    snapshot_ref=ref('outline__users_snapshot'),
    entity_id_col='user_id',
    fields=[
        'name', 'email', 'role', 'is_suspended'
    ]
) }}
