-- depends_on: {{ ref('active_directory__identity_inputs') }}
-- depends_on: {{ ref('active_directory__manager_identity_inputs') }}
-- depends_on: {{ ref('seed_identity_inputs_from_cursor') }}
-- depends_on: {{ ref('outline__identity_inputs') }}
-- depends_on: {{ ref('jira__identity_inputs') }}
-- depends_on: {{ ref('ms_entra__identity_inputs') }}
-- depends_on: {{ ref('bamboohr__identity_inputs') }}
-- depends_on: {{ ref('zulip_proxy__identity_inputs') }}
-- depends_on: {{ ref('zoom__identity_inputs') }}
-- depends_on: {{ ref('github_directory__identity_inputs') }}
-- depends_on: {{ ref('github__identity_inputs') }}
-- depends_on: {{ ref('bitbucket_cloud__identity_inputs') }}
-- depends_on: {{ ref('gitlab__identity_inputs') }}
-- @cpt-principle:cpt-dataflow-principle-rmt-with-version:p1
{{ config(
    materialized='incremental',
    incremental_strategy='delete+insert',
    unique_key='unique_key',
    schema='identity',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['silver']
) }}


-- Named, not `SELECT *`: the union inside is POSITIONAL and takes its column
-- names from whichever contributor lands first, so a producer that adds or
-- reorders a column would silently re-point this relation's columns at other
-- producers' values. Selecting by name here makes that a build failure instead.
-- check-field-parity.py audits the contributors against this shape.
SELECT
    unique_key,
    insight_tenant_id,
    insight_source_id,
    insight_source_type,
    source_account_id,
    value_type,
    value,
    value_field_name,
    operation_type,
    _synced_at,
    _version
FROM (
    {{ union_by_tag('silver:identity_inputs') }}
)
{% if is_incremental() %}
WHERE _version > (SELECT max(_version) FROM {{ this }})
{% endif %}
