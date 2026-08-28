-- depends_on: {{ ref('github__bronze_promoted') }}
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['github']
) }}

-- SCD2 snapshot of every board's field catalogue — appends a version only when
-- a tracked column actually changes.
--
-- GitHub exposes no history for a board field: no timeline event fires when a
-- field is renamed, an option is renamed, or a select gains a column. Bronze
-- therefore holds only what is true now, and this is the only record that it
-- was ever different. `options_json` is tracked as a whole because an option
-- rename is a change to the set, and the set is what a status mapping is
-- authored against.

{{ snapshot(
    source_ref=source('bronze_github', 'project_fields'),
    unique_key_col='unique_key',
    check_cols=[
        'field_name', 'data_type', 'is_multi', 'is_mirror', 'is_issue_field',
        'options_json', 'configuration_json'
    ]
) }}
