{{ config(
    materialized='table',
    schema='silver',
    engine='ReplacingMergeTree',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['silver']
) }}

-- Staging sources tagged `silver:class_people` are discovered at compile
-- time by `union_by_tag` below. The `depends_on` comments declare them
-- to dbt's DAG so they materialise before this model. New connectors
-- adding a `silver:class_people`-tagged staging model MUST add a line
-- here too (project convention is `<source>__to_class_people`).
-- depends_on: {{ ref('bamboohr__to_class_people') }}
-- depends_on: {{ ref('ms_entra__to_class_people') }}

SELECT * FROM (
    {{ union_by_tag('silver:class_people') }}
)
