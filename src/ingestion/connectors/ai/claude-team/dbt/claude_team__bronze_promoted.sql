{# -------------------------------------------------------------------------
   Bootstrap model for claude-team bronze → RMT promotion.

   Counterpart of `cursor__bronze_promoted` for Claude Team. See ADR-0002.
   The `promote_bronze_to_rmt` macro is idempotent — already-RMT tables are
   detected and skipped on subsequent runs.
   ------------------------------------------------------------------------- #}

-- @cpt-principle:cpt-dataflow-principle-promote-bronze:p1
{{ config(
    materialized='view',
    schema='staging',
    tags=['claude-team']
) }}

{% do promote_bronze_to_rmt(table='bronze_claude_team.claude_team_members',      order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_claude_team.claude_team_code_metrics', order_by='unique_key') %}

SELECT 1 AS promoted
