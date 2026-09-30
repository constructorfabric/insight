{% macro youtrack_reconcile_class() %}
    {% if execute and is_incremental() %}
        {% set producer = 'youtrack__' ~ model.name[6:] %}
        {% set relation = adapter.get_relation(database=none, schema='staging', identifier=producer) %}
        {% if relation %}
            DELETE FROM {{ this }}
            WHERE data_source = 'youtrack'
              AND unique_key NOT IN (SELECT unique_key FROM {{ relation }})
            SETTINGS mutations_sync = 2
        {% endif %}
    {% endif %}
{% endmacro %}
