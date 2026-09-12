{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['claude-team', 'silver', 'silver:identity_inputs']
) }}

-- INVARIANT: no deactivation_condition. The only seat-state columns the stream
-- carries (`role`, `seat_tier`) are re-asserted on every read, so a condition
-- over them would emit DELETE rows for every identity field of an account the
-- vendor merely re-described.

{{ identity_inputs_from_history(
    fields_history_ref=ref('claude_team__members_fields_history'),
    source_type='claude-team',
    identity_fields=[
        {'field': 'email_address', 'value_type': 'email',        'value_field_name': 'bronze_claude_team.claude_team_members.account.email_address'},
        {'field': 'full_name',     'value_type': 'display_name', 'value_field_name': 'bronze_claude_team.claude_team_members.account.full_name'},
    ]
) }}
