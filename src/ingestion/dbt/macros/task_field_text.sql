{#-
  The text reference of `silver.class_task_field_history`: a long-text value
  carries `task_text_id(body)` in `value_ids[1]` and `task_text_prefix(body)` in
  `value_displays[1]`, and the body itself lives in `silver.class_task_field_text`
  under the same `text_id`. Every task-tracker producer addresses a body through
  these macros, so one body has one address whichever connector wrote it.
-#}


{#- The `value_id_type` of a text reference. -#}
{% macro task_text_ref_type() %}'text_ref'{% endmacro %}


{#- INVARIANT: a pure function of the body, so the same body always resolves to
    the same `class_task_field_text` row. `sipHash128` is not cryptographic; it
    is a fast, well-distributed 128-bit digest, which is what content addressing
    needs. -#}
{% macro task_text_id(content) %}
    lower(hex(sipHash128({{ content }})))
{% endmacro %}


{#- `substringUTF8`, never `substring`: ClickHouse's `substring` counts bytes, so
    a cut mid-character leaves invalid UTF-8 that a strict reader rejects. -#}
{% macro task_text_prefix(content) %}
    substringUTF8({{ content }}, 1, {{ var('task_long_text_prefix_chars', 200) }})
{% endmacro %}
