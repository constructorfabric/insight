{% macro youtrack_json(expr) -%}
    if(JSONType(coalesce({{ expr }}, 'null')) = 'String',
       JSONExtractString(coalesce({{ expr }}, 'null')),
       coalesce({{ expr }}, 'null'))
{%- endmacro %}

{% macro youtrack_timestamp(expr) -%}
    if(isNotNull(toInt64OrNull(toString({{ expr }}))),
       fromUnixTimestamp64Milli(toInt64OrNull(toString({{ expr }}))),
       parseDateTime64BestEffortOrNull(toString({{ expr }}), 3))
{%- endmacro %}

{% macro youtrack_values(expr) -%}
    arraySort(x -> x.1, arrayDistinct(arrayMap(v -> (
        multiIf(JSONType(v) = 'Object',
                    coalesce(nullIf(JSONExtractString(v, 'id'), ''),
                             nullIf(JSONExtractRaw(v, 'minutes'), ''),
                             nullIf(JSONExtractString(v, 'text'), ''),
                             nullIf(JSONExtractString(v, 'name'), ''), v),
                JSONType(v) = 'String', JSONExtractString(v), v),
        multiIf(JSONType(v) = 'Object',
                    coalesce(nullIf(JSONExtractRaw(v, 'minutes'), ''),
                             nullIf(JSONExtractString(v, 'name'), ''),
                             nullIf(JSONExtractString(v, 'fullName'), ''),
                             nullIf(JSONExtractString(v, 'text'), ''),
                             nullIf(JSONExtractString(v, 'presentation'), ''),
                             nullIf(JSONExtractString(v, 'id'), ''), v),
                JSONType(v) = 'String', JSONExtractString(v), v)
    ), if(JSONType({{ expr }}) = 'Array', JSONExtractArrayRaw({{ expr }}),
          if({{ expr }} IN ('null', ''), CAST([] AS Array(String)), [{{ expr }}])))))
{%- endmacro %}

{% macro youtrack_project_values(expr) -%}
    if({{ expr }} IN ('null', ''), CAST([] AS Array(Tuple(String, String))), [(
        JSONExtractString({{ expr }}, 'project', 'id'),
        coalesce(nullIf(JSONExtractString({{ expr }}, 'project', 'name'), ''),
                 nullIf(JSONExtractString({{ expr }}, 'project', 'shortName'), ''),
                 JSONExtractString({{ expr }}, 'project', 'id'))
    )])
{%- endmacro %}

{% macro youtrack_value_types(expr) -%}
    arrayMap(v -> multiIf(
        JSONExtractString(v, '$type') = 'User', 'account_id',
        JSONType(v) = 'Object' AND JSONExtractString(v, 'id') != '', 'opaque_id',
        JSONType(v) IN ('String', 'Int64', 'UInt64', 'Float64', 'Bool'), 'string_literal',
        'none'),
        if(JSONType({{ expr }}) = 'Array', JSONExtractArrayRaw({{ expr }}),
           if({{ expr }} IN ('null', ''), CAST([] AS Array(String)), [{{ expr }}])))
{%- endmacro %}
