-- depends_on: {{ ref('github__bronze_promoted') }}
{{ config(
    materialized='table',
    schema='staging',
    tags=['github']
) }}

-- Field-level change log of a board's field catalogue, derived from the
-- snapshot: one row per changed attribute per version transition, carrying
-- `old_value` and `new_value`.
--
-- This is what answers "when did that column stop being called X" — the
-- question a status value keyed on a name cannot answer on its own, because
-- the catalogue only ever states the present.
--
-- `entity_id` is the board field's node id, which is globally unique across
-- boards, so it needs no board prefix to stay distinct.

{{ fields_history(
    snapshot_ref=ref('github__project_fields_snapshot'),
    entity_id_col='field_id',
    fields=[
        'field_name', 'data_type', 'is_multi', 'is_mirror', 'is_issue_field',
        'options_json', 'configuration_json'
    ]
) }}
