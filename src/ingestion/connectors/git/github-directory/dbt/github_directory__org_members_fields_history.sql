-- depends_on: {{ ref('github_directory__bronze_promoted') }}
{{ config(
    materialized='table',
    schema='staging',
    tags=['github_directory', 'silver']
) }}

-- Field-level change log of the GitHub member profile, derived from the
-- snapshot. Input to github_directory__identity_inputs.
--
-- entity_id is the LOWERCASED login, not the raw one: it becomes the
-- `value_type='id'` binding that the authenticator matches against the IdP's
-- external-id claim, and that comparison is byte-exact against a
-- case-sensitive column while Keycloak lowercases the brokered username.
-- Mirrors youtrack__users_fields_history.

{{ fields_history(
    snapshot_ref=ref('github_directory__org_members_snapshot'),
    entity_id_col='login_normalized',
    fields=[
        'login', 'name', 'email'
    ]
) }}
