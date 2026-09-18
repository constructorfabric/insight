{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['chatgpt-team', 'silver', 'silver:identity_inputs']
) }}

-- `deactivated_time` is the vendor's departure signal and is tracked in the
-- history, but it is NOT an identity field: it says the account stopped
-- asserting an identity, it is not an identity of its own.
--
-- INVARIANT: deactivation needs two halves, and they do different jobs.
--
-- The condition below is what EMITS the deletes — for every identity field and
-- for the ADR-0002 `id` binding. Clearing a field on its own emits nothing: the
-- macro's field path keeps only `operation_type = 'UPSERT'`, so a value going
-- to '' is dropped rather than published as a delete.
--
-- Blanking email and name in chatgpt_team__seats_latest is what makes the
-- transition REVERSIBLE. The macro re-emits an identity UPSERT only when the
-- field's own value changes, so a condition alone would delete an account's
-- email on deactivation and never restore it when the seat came back with the
-- same address. Blanked on the way out, the return reads as a change from ''
-- back to the address and publishes the UPSERT again.
--
-- `new_value != ''` and not `IS NOT NULL`: the projection normalises every
-- tracked column to '', so an account that was never deactivated reads as
-- empty and a reactivation reads as a change back to empty — neither fires.

{{ identity_inputs_from_history(
    fields_history_ref=ref('chatgpt_team__seats_fields_history'),
    source_type='chatgpt-team',
    identity_fields=[
        {'field': 'email', 'value_type': 'email',        'value_field_name': 'bronze_chatgpt_team.chatgpt_team_seats.email'},
        {'field': 'name',  'value_type': 'display_name', 'value_field_name': 'bronze_chatgpt_team.chatgpt_team_seats.name'},
    ],
    deactivation_condition="field_name = 'deactivated_time' AND new_value != ''"
) }}
