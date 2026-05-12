{{ config(
    materialized='table',
    schema='staging',
    tags=['slack', 'silver']
) }}

{{ fields_history(
    snapshot_ref=ref('slack__users_snapshot'),
    entity_id_col='unique_key',
    fields=[
        'email',
        'is_guest',
        'is_billable_seat'
    ]
) }}
