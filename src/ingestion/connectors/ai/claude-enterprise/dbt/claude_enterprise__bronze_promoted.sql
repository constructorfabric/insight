{# -------------------------------------------------------------------------
   Bootstrap model for Claude Enterprise bronze → RMT promotion.

   Counterpart of `jira__bronze_promoted` for Claude Enterprise. See
   ADR-0002. The `promote_bronze_to_rmt` macro is idempotent — already-RMT
   tables are detected and skipped on subsequent runs.
   ------------------------------------------------------------------------- #}

-- @cpt-principle:cpt-dataflow-principle-promote-bronze:p1
{{ config(
    materialized='view',
    schema='staging',
    tags=['claude-enterprise']
) }}

{% do promote_bronze_to_rmt(table='bronze_claude_enterprise.claude_enterprise_chat_projects', order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_claude_enterprise.claude_enterprise_connectors',    order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_claude_enterprise.claude_enterprise_skills',        order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_claude_enterprise.claude_enterprise_summaries',     order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_claude_enterprise.claude_enterprise_users',         order_by='unique_key') %}

SELECT 1 AS promoted
