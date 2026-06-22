{# -------------------------------------------------------------------------
   Bootstrap model for GitLab bronze -> RMT promotion.

   Counterpart of `bitbucket_cloud__bronze_promoted`. The `promote_bronze_to_rmt`
   macro is idempotent — already-RMT tables are detected and skipped on
   subsequent runs. Only tables that feed a `class_git_*` staging model are
   promoted; the connector's other bronze tables (merge_request_discussions,
   merge_request_state_events, issues, users) have no silver target yet.
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

SELECT 1 AS promoted
