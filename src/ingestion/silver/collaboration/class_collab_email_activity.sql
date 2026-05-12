-- depends_on: {{ ref('m365__collab_email_activity') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    incremental_strategy='append',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='silver',
    tags=['silver']
) }}

SELECT * FROM (
    {{ union_by_tag('silver:class_collab_email_activity') }}
)
{% if is_incremental() %}
WHERE _version > (SELECT max(_version) FROM {{ this }})
{% endif %}
