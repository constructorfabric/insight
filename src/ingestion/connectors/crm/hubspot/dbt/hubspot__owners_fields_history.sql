{{ config(
    materialized='table',
    schema='staging',
    tags=['hubspot']
) }}

{{ fields_history(
    snapshot_ref=ref('hubspot__owners_snapshot'),
    entity_id_col='owner_id',
    fields=['email', 'display_name', 'is_archived']
) }}
