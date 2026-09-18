{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['chatgpt-team', 'silver', 'silver:identity_inputs']
) }}

-- INVARIANT: no deactivation_condition. The roster carries no departure
-- signal — a member absent from a fetch simply stops appearing — so there is
-- no column a condition could read without inventing a tombstone the source
-- never sent.

{{ identity_inputs_from_history(
    fields_history_ref=ref('chatgpt_team__seats_fields_history'),
    source_type='chatgpt-team',
    identity_fields=[
        {'field': 'email', 'value_type': 'email',        'value_field_name': 'bronze_chatgpt_team.chatgpt_team_seats.email'},
        {'field': 'name',  'value_type': 'display_name', 'value_field_name': 'bronze_chatgpt_team.chatgpt_team_seats.name'},
    ]
) }}
