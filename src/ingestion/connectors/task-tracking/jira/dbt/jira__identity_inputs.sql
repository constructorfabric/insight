-- depends_on: {{ ref('jira__bronze_promoted') }}
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['jira', 'silver', 'silver:identity_inputs']
) }}

-- Identity-resolution inputs for the Jira user directory; unioned into
-- silver.identity_inputs via the `silver:identity_inputs` tag. Mirrors
-- bamboohr__identity_inputs / outline__identity_inputs. The canonical
-- `value_type='id'` binding row (source_account_id = Atlassian accountId) is
-- emitted by the macro automatically (ADR-0002).

{{ identity_inputs_from_history(
    fields_history_ref=ref('jira__users_fields_history'),
    source_type='jira',
    identity_fields=[
        {'field': 'email',        'value_type': 'email',        'value_field_name': 'bronze_jira.jira_user.email'},
        {'field': 'display_name', 'value_type': 'display_name', 'value_field_name': 'bronze_jira.jira_user.display_name'},
    ],
    deactivation_condition="field_name = 'active' AND lower(new_value) = 'false'"
) }}
