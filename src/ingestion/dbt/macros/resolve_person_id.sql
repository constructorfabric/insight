{#-
  THE resolve point of the metrics identity rework: collapses the raw
  `identity.identity_persons` observation log (see create_identity_persons /
  the service's persons-sync) into the CURRENT `email -> person_id` map, for
  gold observation models to LEFT JOIN:

      LEFT JOIN ({{ resolve_person_id() }}) AS identity_map
          ON identity_map.email = <entity_id expression>

  Emits (email, person_id), one row per email. Resolution rule v1 —
  latest-observation-wins: the newest `value_type='email'` row per normalized
  email claims it (`created_at DESC, id DESC`; the id tiebreak makes
  same-instant observations deterministic, matching the service's own
  reader ordering). No tenant filter — single-tenant reality (#1550); the
  tenant column is in the log when that changes.

  This macro is deliberately the ONLY place resolution semantics live.
  Future smarts — per-source maps ("this email as seen by git sources"),
  tenant scoping, as_of resolution off `created_at` — change this body and
  every consuming model picks it up on the next build. Consumers must not
  re-derive person_id any other way.

  NORMALIZATION CONTRACT: `lower(trimBoth(...))` on BOTH sides, enforced in
  the join itself — resolved_person_id_join() applies the same expression to
  the model's entity_id rather than trusting each model to have normalized
  identically (they don't: git trims, ai/wiki/task only lowercase, collab
  inherits person_key from the class contract). Idempotent for
  already-normalized keys; for the rest it is the difference between
  resolving and silently missing.

  Dedup note (check-dbt-conventions): identity_persons is a plain MergeTree
  replaced wholesale by an atomic snapshot swap — no ReplacingMergeTree, no
  duplicate row versions to collapse, so no FINAL here; LIMIT 1 BY picks the
  resolution winner, not a dedup survivor.
-#}

{% macro resolve_person_id() %}
    SELECT
        lower(trimBoth(value_effective)) AS email,
        person_id
    FROM identity.identity_persons
    WHERE value_type = 'email'
      AND value_effective IS NOT NULL
      AND trimBoth(value_effective) != ''
    ORDER BY
        email,
        created_at DESC,
        id DESC
    LIMIT 1 BY email
{% endmacro %}

{#-
  Companions for the models that join the map directly (the evidence wrappers
  and the cohort relation): the join and the resolved-check read identically
  across consumers, so grep finds one shape.
-#}

{% macro resolved_person_id_join(rel) %}
    LEFT JOIN ({{ resolve_person_id() }}) AS identity_map
        ON identity_map.email = lower(trimBoth({{ rel }}.entity_id))
{% endmacro %}


{#-
  The canonical serving shape: `entity_type + entity_id` identifies the
  measured entity, and for `person` that identity IS the canonical person id.
  So gold projects the resolved UUID INTO entity_id rather than carrying a
  second identity column beside the source-native email.

      SELECT
          ...,
          {{ canonical_entity_id() }},
          ...
      FROM value_measures
      {{ resolved_person_id_join('value_measures') }}
      WHERE {{ resolved_only() }}
      GROUP BY ..., identity_map.person_id, ...

  Grouped on `identity_map.person_id`, never on the `entity_id` alias: an alias
  that shadows a source column makes ClickHouse substitute it into the outer
  scope (ILLEGAL_AGGREGATION, code 184).

  Text, not UUID: `entity_id` is a String across every entity type, and the
  contract is polymorphic — a UUID column would make the shape person-only.
-#}
{% macro canonical_entity_id() %}
    toString(assumeNotNull(identity_map.person_id)) AS entity_id
{% endmacro %}

{#-
  Keeps unresolved source rows OUT of the canonical relations: with entity_id
  BEING the person id, a row identity cannot resolve has no identity to serve
  under. Nothing is hidden — the pre-resolution evidence relations keep every
  source row, and identity_resolution_coverage measures the gap from there.
-#}
{% macro resolved_only() %}
    identity_map.email != ''
{% endmacro %}

{#-
  Collapsing a person's several source aliases into their one canonical row.

  Additive measures sum: two accounts' commits are that person's commits.
  DAY FLAGS must not. A presence marker is `1` per (alias, day), so summing
  says a person with two accounts had two active days in one day —
  `max` keeps it one.

  `meeting_free_day` is the INVERSE flag: the day is free only if EVERY alias
  was free, so one alias with meetings makes it 0 — `min`, not `max`. Getting
  that backwards would report the busiest people as the most protected.

  Distinct-count measures need nothing here: the runtime counts
  `uniqExact(subject_key)`, so two aliases naming the same subject collapse on
  read. Event-grain rows are not aggregated at all.
-#}
{% macro collapsed_value(expr, max_keys=[], min_keys=[]) %}
    {%- if not max_keys and not min_keys -%}
    sum({{ expr }})
    {%- else -%}
    multiIf(
        {%- if max_keys %}
        measure_key IN ('{{ max_keys | join("', '") }}'), max({{ expr }}),
        {%- endif %}
        {%- if min_keys %}
        measure_key IN ('{{ min_keys | join("', '") }}'), min({{ expr }}),
        {%- endif %}
        sum({{ expr }})
    )
    {%- endif -%}
{% endmacro %}
