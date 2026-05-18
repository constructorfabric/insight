{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='silver',
    engine='ReplacingMergeTree(_version)',
    order_by='(unique_key)',
    settings={'allow_nullable_key': 1},
    tags=['silver']
) }}

-- depends_on: {{ ref('salesforce__crm_contacts') }}
-- depends_on: {{ ref('hubspot__crm_contacts') }}

SELECT * FROM (
    {{ union_by_tag('silver:class_crm_contacts') }}
)
{% if is_incremental() %}
WHERE _version > coalesce((SELECT max(_version) FROM {{ this }}), 0)
{% endif %}
