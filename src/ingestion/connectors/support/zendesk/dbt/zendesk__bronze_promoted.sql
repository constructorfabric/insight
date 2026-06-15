{# -------------------------------------------------------------------------
   Bootstrap model for Zendesk bronze → RMT promotion.

   Counterpart of `hubspot__bronze_promoted` / `chatgpt_team__bronze_promoted`
   (ADR-0002). The `promote_bronze_to_rmt` macro is idempotent: already-RMT
   tables are detected and skipped, and tables Airbyte has NOT created yet are
   skipped too. `support_ticket_events` (the Ticket Audits stream — live as
   stream 4 in connector.yaml since 1.2.0) is promoted here and read by the
   live `zendesk__support_event` silver model.

   `source_zendesk` envelope adds a deterministic `unique_key` to every record,
   so ORDER BY unique_key is the natural-key dedup.
   ------------------------------------------------------------------------- #}

-- @cpt-principle:cpt-dataflow-principle-promote-bronze:p1
{{ config(
    materialized='view',
    schema='staging',
    tags=['zendesk']
) }}

{% do promote_bronze_to_rmt(table='bronze_zendesk.support_tickets',               order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_zendesk.support_agents',                order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_zendesk.zendesk_satisfaction_ratings',  order_by='unique_key') %}
-- Phase-2 actor-attributed audits stream. No-op until support_ticket_events is
-- emitted by the connector; the macro silently skips absent bronze tables.
{% do promote_bronze_to_rmt(table='bronze_zendesk.support_ticket_events',         order_by='unique_key') %}

SELECT 1 AS promoted
