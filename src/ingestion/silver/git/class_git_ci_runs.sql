-- depends_on: {{ ref('github__ci_runs') }}
{{ config(
    materialized='incremental',
    full_refresh=false,
    unique_key='unique_key',
    incremental_strategy='delete+insert',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='silver',
    tags=['silver']
) }}

-- INVARIANT: this table is the only archive of CI history. The source APIs
-- retain runs for a bounded window (GitHub: ~90 days), so a --full-refresh
-- destroys everything older than that window with nothing to rebuild from.
-- Never full-refresh it; heal schema drift with ALTER migrations instead.
SELECT * FROM (
    {{ union_by_tag('silver:class_git_ci_runs') }}
)
{% if is_incremental() %}
WHERE _version > (SELECT max(_version) FROM {{ this }})
{% endif %}
