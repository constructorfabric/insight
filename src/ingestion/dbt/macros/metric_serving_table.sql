{% macro metric_serving_query_settings(join_use_nulls=none) %}
    {% set settings = {
        'max_memory_usage': 2147483648,
        'max_threads': 2,
        'max_block_size': 32768,
        'max_insert_block_size': 32768,
        'min_insert_block_size_rows': 32768,
        'min_insert_block_size_bytes': 16777216,
        'max_partitions_per_insert_block': 512,
        'max_bytes_before_external_group_by': 268435456,
        'max_bytes_before_external_sort': 268435456,
        'max_bytes_in_join': 268435456,
        'join_algorithm': 'auto'
    } %}
    {% if join_use_nulls is not none %}
        {% do settings.update({'join_use_nulls': join_use_nulls}) %}
    {% endif %}
    {{ return(settings) }}
{% endmacro %}

{% macro metric_serving_table(include_record_id, join_use_nulls=none) %}
    {% set order_by = [
        'tenant_id',
        'source_key',
        'entity_type',
        'entity_id',
        'measure_key',
        'metric_date'
    ] %}
    {% if include_record_id %}
        {% do order_by.append('record_id') %}
    {% endif %}
    {{ config(
        materialized='table',
        engine='MergeTree',
        order_by=order_by,
        partition_by='toYYYYMM(metric_date)',
        schema=var('gold_database'),
        tags=['gold'],
        query_settings=metric_serving_query_settings(join_use_nulls=join_use_nulls)
    ) }}
{% endmacro %}

{% macro metric_evidence_table(join_use_nulls=none) %}
    {{ metric_serving_table(true, join_use_nulls=join_use_nulls) }}
{% endmacro %}

{% macro metric_observations_table() %}
    {{ metric_serving_table(false) }}
{% endmacro %}
