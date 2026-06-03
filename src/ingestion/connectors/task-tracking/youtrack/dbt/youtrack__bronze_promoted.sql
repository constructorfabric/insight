{# -------------------------------------------------------------------------
   Bootstrap model for YouTrack bronze → RMT promotion.

   Counterpart of `jira__bronze_promoted`. See ADR-0002. The
   `promote_bronze_to_rmt` macro is idempotent — already-RMT tables are
   detected and skipped, and tables not yet created by Airbyte are skipped.
   Every stream emits a deterministic `unique_key` (primary_key in
   connector.yaml), so ORDER BY unique_key is the natural-key dedup.

   NOTE: YouTrack staging/silver dbt models are not built yet (only the bronze
   source declarations exist). This model promotes bronze pre-emptively so the
   tables are RMT before the first downstream model lands; new YouTrack staging
   models MUST add `-- depends_on: {{ ref('youtrack__bronze_promoted') }}`.
   ------------------------------------------------------------------------- #}

-- @cpt-principle:cpt-dataflow-principle-promote-bronze:p1
{{ config(
    materialized='view',
    schema='staging',
    tags=['youtrack']
) }}

{% do promote_bronze_to_rmt(table='bronze_youtrack.youtrack_projects',             order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_youtrack.youtrack_user',                 order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_youtrack.youtrack_agiles',               order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_youtrack.youtrack_sprints',              order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_youtrack.youtrack_issue_link_types',     order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_youtrack.youtrack_issue',                order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_youtrack.youtrack_issue_history',        order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_youtrack.youtrack_comments',             order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_youtrack.youtrack_worklogs',             order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_youtrack.youtrack_project_custom_fields', order_by='unique_key') %}

SELECT 1 AS promoted
