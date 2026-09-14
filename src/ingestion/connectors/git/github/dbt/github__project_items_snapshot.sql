-- depends_on: {{ ref('github__bronze_promoted') }}
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['github']
) }}

-- SCD2 snapshot of board cards — appends a version only when the card's
-- tracked state changes.
--
-- Status history comes from the issue timeline and does NOT depend on this
-- model. Every OTHER board field value does: no API exposes their history, and
-- `updatedAt` says when a value was last touched but never what it was before.
-- Successive versions here are the only way to see one change.
--
-- `field_values_json` is tracked whole, so a version appears when any board
-- field on the card moves. Splitting that into one row per (card, field)
-- belongs in the model that first has a consumer for it — pinning the value
-- shape now would fix a contract nothing reads yet.

{{ snapshot(
    source_ref=source('bronze_github', 'project_items'),
    unique_key_col='unique_key',
    check_cols=[
        'is_archived', 'content_type', 'content_number', 'content_repo_full_name',
        'field_values_json'
    ]
) }}
