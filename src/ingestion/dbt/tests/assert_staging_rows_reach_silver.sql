{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'every staging row reaches its silver class',
        'domain': 'silver',
        'category': 'completeness',
        'tier': 'error',
        'remediation': 'A staging row a silver class never admitted is lost for good — nothing re-reads it. The usual cause is an incremental boundary that does not separate the producer: one producer''s run raised it past another producer''s rows. Check the class model passes silver_incremental_watermark keys that tell every producer apart (scripts/ci/incremental_boundary.py checks the keys exist, not that they suffice). The rows are still in staging, so recovery is a re-read of the union — a full refresh of the class, or an anti-join insert from staging.'
    }
) }}

{#- One row per (class, producer) whose staging rows did not arrive. Every
    incremental model built by union_by_tag is a class; a class or producer this
    deployment has not materialised is skipped rather than reported empty.
    Relations, not ref(): the classes are discovered at run time, and a ref()
    dbt cannot see at parse time fails compilation. Keys are compared as
    hashes so the anti-join holds 8 bytes per row. -#}
{%- set branches = [] -%}
{%- if execute -%}
  {%- set models = graph.nodes.values() | selectattr('resource_type', 'equalto', 'model') | list -%}
  {%- for node in models if node.config.materialized == 'incremental' -%}
    {%- set found = modules.re.search("union_by_tag\\(\\s*'([^']+)'", node.raw_code) -%}
    {%- set served = adapter.get_relation(database=none, schema=node.schema, identifier=node.alias or node.name) if found else none -%}
    {%- if served -%}
      {%- set staged = [] -%}
      {%- for producer in models if found.group(1) in producer.tags and producer.unique_id != node.unique_id
                                    and producer.config.materialized != 'ephemeral' -%}
        {%- set rel = adapter.get_relation(database=none, schema=producer.schema, identifier=producer.alias or producer.name) -%}
        {%- if rel -%}{%- do staged.append((producer.name, rel)) -%}{%- endif -%}
      {%- endfor -%}
      {%- if staged | length > 0 -%}{%- do branches.append((node.name, served, staged)) -%}{%- endif -%}
    {%- endif -%}
  {%- endfor -%}
{%- endif -%}

{% if branches | length == 0 %}
SELECT
    CAST('' AS String)  AS class_name,
    CAST('' AS String)  AS producer,
    toUInt64(0)         AS missing_rows
WHERE 1 = 0
{% else %}
{% for class_name, served, staged in branches %}
{% for producer, rel in staged %}
SELECT
    '{{ class_name }}'  AS class_name,
    '{{ producer }}'    AS producer,
    count()             AS missing_rows
FROM (SELECT DISTINCT cityHash64(unique_key) AS k FROM {{ rel }}) AS staged
LEFT ANTI JOIN (SELECT cityHash64(unique_key) AS k FROM {{ served }}) AS served USING (k)
HAVING missing_rows > 0
{% if not loop.last %}UNION ALL{% endif %}
{% endfor %}
{% if not loop.last %}UNION ALL{% endif %}
{% endfor %}
{% endif %}
