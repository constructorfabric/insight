{% macro insight_uuid_from_raw(column) %}
{#-
  The project-wide mapping from a raw connector-config identifier (tenant id,
  source instance id) to the UUID the identity relations carry.

  Identity relations type these columns as UUID so the silver UNION ALL
  type-checks (ClickHouse rejects UNION across UUID and String, NO_COMMON_TYPE),
  while connector configuration supplies free-form strings — so the raw value
  is hashed into a UUID. The same raw string always maps to the same UUID, which
  is what keeps cross-source joins consistent.

  This is the JOIN RECIPE for anything keyed by raw strings — the person
  attribute claim relations and the attribute policy snapshot both carry raw
  identifiers, so joining them to an identity relation means hashing the raw
  side with this macro. Writing the expression by hand is how a join silently
  returns nothing: a different hash width or a missing coalesce still compiles
  and still produces a UUID, just not the same one.

  TEMPORARY, pending a real tenants registry that issues actual UUIDs; when
  that lands this macro is the single place the mapping is retired from.
-#}
toUUID(UUIDNumToString(sipHash128(coalesce({{ column }}, ''))))
{% endmacro %}
