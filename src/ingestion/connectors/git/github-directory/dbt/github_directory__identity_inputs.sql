{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['github-directory', 'silver', 'silver:identity_inputs']
) }}

-- Identity-resolution inputs for the GitHub account roster; unioned into
-- silver.identity_inputs via the `silver:identity_inputs` tag. Mirrors
-- youtrack__identity_inputs.
--
-- source_type is 'github', not 'github-directory': it must equal the
-- authenticator's `idp.source_type`, which names the vendor a person
-- authenticated against, not the connector package that supplied the roster.
--
-- The canonical `value_type='id'` binding row is emitted by the macro from
-- entity_id (= the member's immutable numeric GitHub id, stringified) — that
-- is the row the login lookup matches, so the broker must put the same
-- numeric id in the external-id claim, as a JSON string.
-- `email` carries a member's org email where one is verified
-- and visible to the token's scopes; where it is absent, resolution leans on
-- the id binding and display_name.
--
-- No deactivation condition applies, and none can be expressed here: the
-- macro's DELETE branch selects `FROM history WHERE <condition>`, so it fires
-- only on a CHANGE. A member removed from the org produces no change at all —
-- they stop appearing in the roster, their last bronze row survives under
-- ReplacingMergeTree, and the snapshot therefore emits nothing to match on.
-- Detecting removal needs a roster diff across syncs, which is a new model
-- rather than a predicate. The condition below is intentionally unsatisfiable
-- so the branch stays wired for whatever supplies that signal.
--
-- Consequence until then: a departed member keeps an active GitHub binding.
-- Revocation belongs at the IdP, which is the only control that takes effect
-- when it happens rather than at the next sync — a daily batch cannot be an
-- access-revocation mechanism at any fidelity. Do not treat this connector as
-- one.

{{ identity_inputs_from_history(
    fields_history_ref=ref('github_directory__org_members_fields_history'),
    source_type='github',
    identity_fields=[
        {'field': 'email', 'value_type': 'email',        'value_field_name': 'bronze_github_directory.org_members.email'},
        {'field': 'login', 'value_type': 'username',     'value_field_name': 'bronze_github_directory.org_members.login'},
        {'field': 'name',  'value_type': 'display_name', 'value_field_name': 'bronze_github_directory.org_members.name'},
    ],
    deactivation_condition="field_name = 'login' AND new_value = ''"
) }}
