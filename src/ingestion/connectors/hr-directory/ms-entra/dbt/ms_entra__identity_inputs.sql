{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['ms-entra', 'silver', 'silver:identity_inputs']
) }}

{# Emit identity signals from the MS Entra user directory.

   `userPrincipalName` and `mail` both yield value_type='email' rows so the
   Identity Manager can match a person across services regardless of which
   email form a downstream connector recorded. `proxyAddresses` and
   `otherMails` are stored as arrays in Bronze; the macro operates on
   scalar history rows and they're added separately in a follow-up
   (REC-IR-07: array-valued identity inputs).

   `onPremisesSamAccountName` carries the legacy AD/SAM login — required to
   match users synced from on-premises AD (Entra Connect) against
   Bitbucket Server / GitLab self-hosted, where SAM is often the username.
#}

{{ identity_inputs_from_history(
    fields_history_ref=ref('ms_entra__users_fields_history'),
    source_type='ms-entra',
    identity_fields=[
        {'field': 'mail',                     'value_type': 'email',        'value_field_name': 'bronze_ms_entra.users.mail'},
        {'field': 'userPrincipalName',        'value_type': 'email',        'value_field_name': 'bronze_ms_entra.users.userPrincipalName'},
        {'field': 'employeeId',               'value_type': 'employee_id',  'value_field_name': 'bronze_ms_entra.users.employeeId'},
        {'field': 'displayName',              'value_type': 'display_name', 'value_field_name': 'bronze_ms_entra.users.displayName'},
        {'field': 'onPremisesSamAccountName', 'value_type': 'sam_account',  'value_field_name': 'bronze_ms_entra.users.onPremisesSamAccountName'},
    ],
    deactivation_condition="field_name = 'accountEnabled' AND new_value = 'false'"
) }}
