{# -------------------------------------------------------------------------
   Bootstrap model for Salesforce bronze → RMT promotion.

   Counterpart of `jira__bronze_promoted`. See ADR-0002. The
   `promote_bronze_to_rmt` macro is idempotent — already-RMT tables are
   detected and skipped, and tables not yet created by Airbyte are skipped.
   `source_salesforce/streams.py` adds a deterministic `unique_key` to every
   emitted record, so ORDER BY unique_key is the natural-key dedup.
   ------------------------------------------------------------------------- #}

-- @cpt-principle:cpt-dataflow-principle-promote-bronze:p1
{{ config(
    materialized='view',
    schema='staging',
    tags=['salesforce']
) }}

{% do promote_bronze_to_rmt(table='bronze_salesforce.Account',                 order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_salesforce.Contact',                 order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_salesforce.Opportunity',             order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_salesforce.OpportunityHistory',      order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_salesforce.Task',                    order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_salesforce.Event',                   order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_salesforce.User',                    order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_salesforce.Lead',                    order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_salesforce.Case',                    order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_salesforce.OpportunityContactRole',  order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_salesforce.OpportunityLineItem',     order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_salesforce.CampaignMember',          order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_salesforce.Campaign',                order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_salesforce.Product2',                order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_salesforce.Pricebook2',              order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_salesforce.PricebookEntry',          order_by='unique_key') %}

SELECT 1 AS promoted
