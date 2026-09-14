{# -------------------------------------------------------------------------
   Bootstrap model for GitLab bronze -> RMT promotion.

   The `promote_bronze_to_rmt` macro is idempotent — already-RMT tables are
   detected and skipped on subsequent runs. Every bronze table the connector
   writes is promoted so read-time dedup by `unique_key` is well-defined.
   ------------------------------------------------------------------------- #}

-- @cpt-principle:cpt-dataflow-principle-promote-bronze:p1
{{ config(
    materialized='view',
    schema='staging',
    tags=['gitlab']
) }}

{% do promote_bronze_to_rmt(table='bronze_gitlab.repositories',              order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_gitlab.branches',                  order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_gitlab.commits',                   order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_gitlab.file_changes',              order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_gitlab.commit_authors',            order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_gitlab.pull_requests',             order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_gitlab.pull_request_diff_stats',   order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_gitlab.pull_request_notes',        order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_gitlab.pull_request_commits',      order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_gitlab.pull_request_state_events', order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_gitlab.pull_request_label_events', order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_gitlab.pipelines',                 order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_gitlab.environments',              order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_gitlab.deployments',               order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_gitlab.group_members',             order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_gitlab.users',                     order_by='unique_key') %}

SELECT 1 AS promoted
