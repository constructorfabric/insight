{# -------------------------------------------------------------------------
   Bootstrap model for claude-team-invoices bronze → RMT promotion.

   Counterpart of `claude_team__bronze_promoted` for the invoice connector,
   which owns its own bronze namespace. See ADR-0002. The
   `promote_bronze_to_rmt` macro is idempotent — already-RMT tables are
   detected and skipped on subsequent runs.
   ------------------------------------------------------------------------- #}

-- @cpt-principle:cpt-dataflow-principle-promote-bronze:p1
{{ config(
    materialized='view',
    schema='staging',
    tags=['claude-team-invoices']
) }}

{% do promote_bronze_to_rmt(table='bronze_claude_team_invoices.claude_team_invoice_lines', order_by='unique_key') %}

SELECT 1 AS promoted
