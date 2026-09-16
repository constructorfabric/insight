{{ config(
    materialized='table',
    schema='staging',
    tags=['claude-team', 'silver']
) }}

{{ fields_history(
    snapshot_ref=ref('claude_team__members_snapshot'),
    entity_id_col='account_uuid',
    fields=[
        'email_address', 'full_name'
    ]
) }}
