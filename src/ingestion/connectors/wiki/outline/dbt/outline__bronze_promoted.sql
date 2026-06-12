{# -------------------------------------------------------------------------
   Bootstrap model for Outline bronze → RMT promotion.

   Counterpart of `confluence__bronze_promoted` for Outline. See ADR-0002. The
   `promote_bronze_to_rmt` macro is idempotent — already-RMT tables are
   detected and skipped on subsequent runs.
   ------------------------------------------------------------------------- #}

-- @cpt-principle:cpt-dataflow-principle-promote-bronze:p1
{{ config(
    materialized='view',
    schema='staging',
    tags=['outline']
) }}

{% do promote_bronze_to_rmt(table='bronze_outline.wiki_spaces',        order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_outline.wiki_pages',         order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_outline.wiki_page_versions', order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_outline.wiki_comments',      order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_outline.wiki_users',         order_by='unique_key') %}

SELECT 1 AS promoted
