{{ config(
    materialized='table',
    schema='staging',
    tags=['zulip_proxy', 'silver']
) }}

{{ fields_history(
    snapshot_ref=ref('zulip_proxy__users_snapshot'),
    entity_id_col='id',
    fields=[
        'full_name', 'email', 'is_active', 'role', 'recipient_id', 'uuid'
    ]
) }}
