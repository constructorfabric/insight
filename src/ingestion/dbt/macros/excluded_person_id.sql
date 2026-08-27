{#-
  The reserved person meaning "not a human" (bots, CI, service accounts).
  Unmintable: UUIDv7 never produces an all-ones value.
-#}

{% macro excluded_person_id() %}
    toUUID('ffffffff-ffff-ffff-ffff-ffffffffffff')
{% endmacro %}
