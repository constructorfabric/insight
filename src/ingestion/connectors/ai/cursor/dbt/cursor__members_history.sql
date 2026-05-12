-- Field-level change log for cursor team members
-- One row per changed field per version transition
{{ config(
    materialized='incremental',
    schema='staging',
    tags=['cursor'],
    inserts_only=true
) }}

{{ fields_history(
    snapshot_ref=ref('cursor__members_snapshot'),
    entity_id_col='id',
    fields=['name', 'role', 'isRemoved']
) }}
