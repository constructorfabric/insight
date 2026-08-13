{# -------------------------------------------------------------------------
   Bootstrap model for GitHub bronze -> RMT promotion.

   Counterpart of `gitlab__bronze_promoted`. The `promote_bronze_to_rmt` macro
   is idempotent — already-RMT tables are detected and skipped on subsequent
   runs. Every bronze table the connector writes is promoted so read-time dedup
   by `unique_key` is well-defined, including the tables that do not yet feed a
   `class_git_*` staging model (issues, projects_v2, CI and deployments).
   ------------------------------------------------------------------------- #}

-- @cpt-principle:cpt-dataflow-principle-promote-bronze:p1
{{ config(
    materialized='view',
    schema='staging',
    tags=['github']
) }}

{% do promote_bronze_to_rmt(table='bronze_github.repositories',                order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_github.branches',                    order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_github.commits',                     order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_github.file_changes',                order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_github.pull_requests',               order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_github.pull_request_reviews',        order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_github.pull_request_commits',        order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_github.pull_request_diff_stats',     order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_github.pull_request_comments',       order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_github.pull_request_review_comments', order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_github.issues',                      order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_github.projects_v2',                 order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_github.workflow_runs',               order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_github.deployments',                 order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_github.deployment_statuses',         order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_github.pull_request_timeline_events', order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_github.issue_timeline_events',       order_by='unique_key') %}

SELECT 1 AS promoted
