{#-
  The opaque evidence key of a source account: what a gold model keys a row on
  when the source reported no email. A lowercase hex hash of the raw triple,
  so two accounts that differ only by case stay apart and nothing parses it;
  never an address, so the email map cannot match it and the account triple
  carried beside it is what resolves the row.
-#}
{% macro account_entity_key(source_type_col, source_id_col, account_id_col) -%}
    concat('account:', lower(hex(sipHash128(concat({{ source_type_col }}, '\0', {{ source_id_col }}, '\0', {{ account_id_col }})))))
{%- endmacro %}
