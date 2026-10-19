{#- pre_hook of a silver class: deletes the rows of `data_source` its staging
    producer no longer holds. Only for a producer rebuilt whole every run — a
    row it stops emitting is then gone at the source, not merely unread. -#}
{% macro reconcile_class_from_producer(data_source) %}
    {% if execute and is_incremental() %}
        {% set producer = data_source ~ '__' ~ model.name[6:] %}
        {% set relation = adapter.get_relation(database=none, schema='staging', identifier=producer) %}
        {% if relation %}
            DELETE FROM {{ this }}
            WHERE data_source = '{{ data_source }}'
              AND unique_key NOT IN (SELECT unique_key FROM {{ relation }})
            SETTINGS mutations_sync = 2
        {% endif %}
    {% endif %}
{% endmacro %}
