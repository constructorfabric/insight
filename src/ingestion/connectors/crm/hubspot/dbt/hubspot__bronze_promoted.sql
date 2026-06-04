{# -------------------------------------------------------------------------
   Bootstrap model for HubSpot bronze → RMT promotion.

   Counterpart of `jira__bronze_promoted`. See ADR-0002. The
   `promote_bronze_to_rmt` macro is idempotent — already-RMT tables are
   detected and skipped, and tables not yet created by Airbyte are skipped
   (so absent `*_archived` siblings are no-ops until backfill is enabled).
   `source_hubspot/envelope.py` adds a deterministic `unique_key` to every
   emitted record, so ORDER BY unique_key is the natural-key dedup.
   ------------------------------------------------------------------------- #}

-- @cpt-principle:cpt-dataflow-principle-promote-bronze:p1
{{ config(
    materialized='view',
    schema='staging',
    tags=['hubspot']
) }}

{% do promote_bronze_to_rmt(table='bronze_hubspot.contacts',                    order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_hubspot.contacts_archived',           order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_hubspot.companies',                   order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_hubspot.companies_archived',          order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_hubspot.deals',                       order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_hubspot.deals_archived',              order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_hubspot.owners',                      order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_hubspot.owners_archived',             order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_hubspot.engagements_calls',           order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_hubspot.engagements_calls_archived',  order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_hubspot.engagements_emails',          order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_hubspot.engagements_emails_archived', order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_hubspot.engagements_meetings',        order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_hubspot.engagements_tasks',           order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_hubspot.engagements_tasks_archived',  order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_hubspot.leads',                       order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_hubspot.leads_archived',              order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_hubspot.tickets',                     order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_hubspot.tickets_archived',            order_by='unique_key') %}

SELECT 1 AS promoted
