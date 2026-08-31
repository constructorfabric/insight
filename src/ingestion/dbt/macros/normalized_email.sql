{#-
  INVARIANT: every producer of a person-keyed fact and every email claim in
  `person_map` normalize through this macro. A divergence makes rows silently
  unresolvable — no error, just a person missing activity.
-#}

{% macro normalized_email(expr) %}
    lower(trimBoth({{ expr }}))
{% endmacro %}
