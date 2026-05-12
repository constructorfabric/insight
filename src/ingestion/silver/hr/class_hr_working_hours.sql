{{ config(
    materialized='table',
    schema='silver',
    engine='ReplacingMergeTree',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['silver']
) }}

-- depends_on: {{ ref('bamboohr__working_hours') }}

SELECT * FROM (
    {{ union_by_tag('silver:class_hr_working_hours') }}
)
