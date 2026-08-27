{# -------------------------------------------------------------------------
   Bootstrap model for Compass bronze -> RMT promotion.

   Airbyte writes bronze append-only, so every read needs deduplication by
   `unique_key`. Promotion is what makes a later `FINAL` read well-defined —
   in particular for `deployment_events`, where one event is rewritten in place
   as the deployment progresses and the latest version is the correct one.

   The `promote_bronze_to_rmt` macro is idempotent: already-RMT tables are
   detected and skipped on subsequent runs.
   ------------------------------------------------------------------------- #}

{{ config(
    materialized='view',
    schema='staging',
    tags=['compass']
) }}

{% set compass_tables = [
    'components',
    'scorecards',
    'component_scorecard_scores',
    'teams',
    'team_members',
    'deployment_events',
] %}

{% for table in compass_tables %}
  {% do promote_bronze_to_rmt(table='bronze_compass.' ~ table, order_by='unique_key') %}
{% endfor %}

SELECT 1 AS promoted
