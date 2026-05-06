{# -------------------------------------------------------------------------
   Bootstrap model for Bitbucket Cloud bronze → RMT promotion.

   Counterpart of `jira__bronze_promoted` for Bitbucket Cloud. See ADR-0002.
   The `promote_bronze_to_rmt` macro is idempotent — already-RMT tables are
   detected and skipped on subsequent runs.
   ------------------------------------------------------------------------- #}

-- @cpt-principle:cpt-dataflow-principle-promote-bronze:p1
{{ config(
    materialized='view',
    schema='staging',
    tags=['bitbucket-cloud']
) }}

{% do promote_bronze_to_rmt(table='bronze_bitbucket_cloud.branches',              order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_bitbucket_cloud.commits',               order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_bitbucket_cloud.file_changes',          order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_bitbucket_cloud.pull_requests',         order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_bitbucket_cloud.pull_request_comments', order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_bitbucket_cloud.pull_request_commits',  order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_bitbucket_cloud.repositories',          order_by='unique_key') %}

SELECT 1 AS promoted
