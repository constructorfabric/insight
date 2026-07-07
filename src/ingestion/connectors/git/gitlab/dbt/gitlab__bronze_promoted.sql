{# -------------------------------------------------------------------------
   Bootstrap model for GitLab bronze -> RMT promotion.

   Counterpart of `bitbucket_cloud__bronze_promoted`. The `promote_bronze_to_rmt`
   macro is idempotent — already-RMT tables are detected and skipped on
   subsequent runs. Every full-refresh bronze table the connector writes is
   promoted so read-time dedup by `unique_key` (via FINAL) is well-defined,
   including tables that do not yet feed a `class_git_*` staging model
   (issues, merge_request_discussions, merge_request_state_events).
   ------------------------------------------------------------------------- #}

-- @cpt-principle:cpt-dataflow-principle-promote-bronze:p1
{{ config(
    materialized='view',
    schema='staging',
    tags=['gitlab']
) }}

{% do promote_bronze_to_rmt(table='bronze_gitlab.projects',                order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_gitlab.branches',                order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_gitlab.commits',                 order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_gitlab.commit_file_changes',     order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_gitlab.merge_requests',          order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_gitlab.merge_request_commits',   order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_gitlab.merge_request_notes',     order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_gitlab.merge_request_approvals', order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_gitlab.users',                   order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_gitlab.issues',                          order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_gitlab.merge_request_discussions',       order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_gitlab.merge_request_state_events',      order_by='unique_key') %}

SELECT 1 AS promoted
