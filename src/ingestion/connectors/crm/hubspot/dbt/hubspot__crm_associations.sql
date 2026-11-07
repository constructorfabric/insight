{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by='(unique_key)',
    settings={'allow_nullable_key': 1},
    tags=['hubspot', 'silver:class_crm_associations']
) }}

-- One row per (record, associated object type) holding the full id list, so
-- a removed association disappears with the next version of its record.
{%- set sources = [
    {'table': 'contacts', 'archived': true, 'from': 'contacts', 'to': ['companies', 'deals']},
    {'table': 'deals', 'archived': true, 'from': 'deals', 'to': ['companies', 'contacts']},
    {'table': 'engagements_calls', 'archived': true, 'from': 'calls', 'to': ['contacts', 'companies', 'deals']},
    {'table': 'engagements_emails', 'archived': true, 'from': 'emails', 'to': ['contacts', 'companies', 'deals']},
    {'table': 'engagements_meetings', 'archived': false, 'from': 'meetings', 'to': ['contacts', 'companies', 'deals']},
    {'table': 'engagements_tasks', 'archived': true, 'from': 'tasks', 'to': ['contacts', 'companies', 'deals']},
    {'table': 'engagements_notes', 'archived': true, 'from': 'notes', 'to': ['contacts', 'companies', 'deals']},
    {'table': 'engagements_communications', 'archived': false, 'from': 'communications', 'to': ['contacts', 'companies', 'deals']},
    {'table': 'leads', 'archived': true, 'from': 'leads', 'to': ['contacts', 'companies']},
    {'table': 'tickets', 'archived': true, 'from': 'tickets', 'to': ['contacts', 'companies', 'deals']},
] -%}
{%- set arms = [] -%}
{%- for src in sources -%}
  {%- for tbl in hubspot_bronze_tables(src.table, archived=src.archived) -%}
    {%- for to in src.to -%}
      {%- do arms.append({'table': tbl, 'from': src.from, 'to': to}) -%}
    {%- endfor -%}
  {%- endfor -%}
{%- endfor %}

WITH src AS (
    {% for arm in arms %}
    SELECT
        tenant_id,
        source_id,
        concat(tenant_id, '-', source_id, '-{{ arm.from }}-', id, '-{{ arm.to }}') AS unique_key,
        '{{ arm.from }}'                                AS from_object_type,
        id                                              AS from_object_id,
        '{{ arm.to }}'                                  AS to_object_type,
        JSONExtract(coalesce(associations_{{ arm.to }}, '[]'), 'Array(String)') AS to_object_ids,
        toUInt8(coalesce(archived, false))              AS is_deleted,
        data_source,
        {{ hubspot_record_version() }}                  AS _version
    FROM {{ source('bronze_hubspot', arm.table) }}
    {% if not loop.last %}UNION ALL{% endif %}
    {% endfor %}
)

{{ hubspot_staging_tail('src') }}
