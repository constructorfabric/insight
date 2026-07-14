{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['youtrack', 'silver', 'silver:identity_inputs']
) }}

-- Identity-resolution inputs for the YouTrack user directory; unioned into
-- silver.identity_inputs via the `silver:identity_inputs` tag. Mirrors
-- jira__identity_inputs / outline__identity_inputs. The canonical
-- `value_type='id'` binding row (source_account_id = YouTrack user id) is
-- emitted by the macro automatically (ADR-0002). `email` may be null under Hub
-- privacy — resolution then leans on the id binding + display_name.

{{ identity_inputs_from_history(
    fields_history_ref=ref('youtrack__users_fields_history'),
    source_type='youtrack',
    identity_fields=[
        {'field': 'email',     'value_type': 'email',        'value_field_name': 'bronze_youtrack.youtrack_user.email'},
        {'field': 'login',     'value_type': 'username',     'value_field_name': 'bronze_youtrack.youtrack_user.login'},
        {'field': 'full_name', 'value_type': 'display_name', 'value_field_name': 'bronze_youtrack.youtrack_user.full_name'},
    ],
    deactivation_condition="field_name = 'banned' AND lower(new_value) = 'true'"
) }}
