-- @cpt-principle:cpt-dataflow-principle-rmt-with-version:p1
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='identity',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['silver']
) }}

-- depends_on: {{ ref('bamboohr__identity_inputs') }}
-- depends_on: {{ ref('zoom__identity_inputs') }}
-- depends_on: {{ ref('seed_identity_inputs_from_cursor') }}
-- depends_on: {{ ref('seed_identity_inputs_from_claude_admin') }}

SELECT * FROM (
    {{ union_by_tag('silver:identity_inputs') }}
)
{% if is_incremental() %}
WHERE _version > (SELECT max(_version) FROM {{ this }})
{% endif %}
