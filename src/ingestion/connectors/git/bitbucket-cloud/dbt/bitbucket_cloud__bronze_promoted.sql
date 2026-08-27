{# -------------------------------------------------------------------------
   Bootstrap model for Bitbucket bronze -> RMT promotion.

   The `promote_bronze_to_rmt` macro is idempotent — already-RMT tables are
   detected and skipped on subsequent runs. Every bronze table the connector
   writes is promoted so read-time dedup by `unique_key` is well-defined,
   including the tables that do not yet feed a `class_git_*` staging model
   (pipelines, deployments, workspace members).
   ------------------------------------------------------------------------- #}

-- @cpt-principle:cpt-dataflow-principle-promote-bronze:p1
{{ config(
    materialized='view',
    schema='staging',
    tags=['bitbucket-cloud']
) }}

{% do promote_bronze_to_rmt(table='bronze_bitbucket_cloud.repositories',           order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_bitbucket_cloud.branches',               order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_bitbucket_cloud.commits',                order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_bitbucket_cloud.file_changes',           order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_bitbucket_cloud.pull_requests',          order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_bitbucket_cloud.pull_request_comments',  order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_bitbucket_cloud.pull_request_commits',   order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_bitbucket_cloud.pull_request_diffstat',  order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_bitbucket_cloud.pull_request_activity',  order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_bitbucket_cloud.workspace_members',      order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_bitbucket_cloud.pipelines',              order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_bitbucket_cloud.deployments',            order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_bitbucket_cloud.commit_authors',        order_by='unique_key') %}

SELECT 1 AS promoted
