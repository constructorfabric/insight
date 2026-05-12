{# -------------------------------------------------------------------------
   Bootstrap model for Confluence bronze → RMT promotion.

   Counterpart of `jira__bronze_promoted` for Confluence. See ADR-0002. The
   `promote_bronze_to_rmt` macro is idempotent — already-RMT tables are
   detected and skipped on subsequent runs.
   ------------------------------------------------------------------------- #}

-- @cpt-principle:cpt-dataflow-principle-promote-bronze:p1
{{ config(
    materialized='view',
    schema='staging',
    tags=['confluence']
) }}

{% do promote_bronze_to_rmt(table='bronze_confluence.wiki_pages',         order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_confluence.wiki_page_versions', order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_confluence.wiki_spaces',        order_by='unique_key') %}

SELECT 1 AS promoted
