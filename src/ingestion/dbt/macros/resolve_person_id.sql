{#-
  THE resolve point of the metrics identity rework: collapses the raw
  `identity.identity_persons` observation log (see create_identity_persons /
  the service's persons-sync) into the CURRENT `email -> person_id` map, for
  gold observation models to LEFT JOIN:

      LEFT JOIN ({{ resolve_person_id() }}) AS identity_map
          ON identity_map.email = <entity_id expression>

  Emits (email, person_id), one row per email. The claim is ACCOUNT-DERIVED:
  an email resolves through the accounts that carry it, not through a
  standalone email observation. Every account's current binding is the latest
  `value_type='id'` row for it; an email's claimants are the persons those
  accounts are bound to; the email resolves only when they agree.

  EVERY email an account has carried claims it, not just its current one: a
  person who changed address still owns the facts recorded under the old one,
  and dropping them would unresolve their history at the moment they renamed.
  An address later carried by someone else's account has two claimants and so
  resolves to nobody — recycling an address costs resolution, never a wrong
  attribution.

  Why not latest-email-wins (rule v1): operator corrections are recorded as
  bindings — `value_type='id'` rows — so a merge or a detach left the email
  map untouched and never reached the metrics. Reading the map through
  bindings is what makes a correction re-attribute activity on the next build.
  It also makes a shared value fail safe: an email carried by accounts of two
  different people has two claimants and resolves to NOBODY, instead of the
  latest observation silently awarding it to one of them.

  Accounts the source has deactivated still claim: closure means the account
  is gone from the source, not that its history stops belonging to the person.
  Excluded accounts (bots, CI, service accounts — bound to the reserved
  excluded person) claim nothing, so their emails resolve to no one.

  No tenant filter — single-tenant reality (#1550); the tenant column is in
  the log when that changes. And no join on tenant between the two stores:
  `identity_inputs` carries a producer-side hashed tenant that never equals
  the journal's, so the account triple is the only sound key across them.

  This macro is deliberately the ONLY place resolution semantics live.
  Future smarts — per-source maps ("this email as seen by git sources"),
  tenant scoping, as_of resolution off `created_at` — change this body and
  every consuming model picks it up on the next build. Consumers must not
  re-derive person_id any other way.

  NOT YET account-first: gold facts carry an email, never the account that
  produced it (`entity_id` is an email everywhere — see the evidence models),
  so there is nothing to key an account-first lookup on. Propagating source
  account ids through silver is its own piece of work; until then this map is
  how corrections reach gold.

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
{%- set evidence = adapter.get_relation(
        database=target.database, schema='identity', identifier='identity_inputs') -%}
{%- if evidence is none -%}
    {#- The connector evidence has never been built: resolve nothing rather
        than fail the build, exactly as an empty journal would. -#}
    SELECT
        ''                                   AS email,
        toUUID('00000000-0000-0000-0000-000000000000') AS person_id
    WHERE 0
{%- else -%}
    SELECT
        ae.email                             AS email,
        any(cb.person_id)                    AS person_id
    FROM (
        SELECT DISTINCT
            insight_source_type              AS source_type,
            insight_source_id                AS source_id,
            source_account_id                AS account_id,
            lower(trimBoth(value))           AS email
        FROM identity.identity_inputs
        WHERE value_type = 'email'
          AND operation_type = 'UPSERT'
          AND coalesce(value, '') != ''
          AND coalesce(source_account_id, '') != ''
    ) AS ae
    INNER JOIN (
        SELECT
            insight_source_type              AS source_type,
            insight_source_id                AS source_id,
            trimBoth(value_effective)        AS account_id,
            person_id
        FROM identity.identity_persons
        WHERE value_type = 'id'
          AND value_effective IS NOT NULL
          AND trimBoth(value_effective) != ''
        ORDER BY
            source_type,
            source_id,
            account_id,
            created_at DESC,
            id DESC
        LIMIT 1 BY source_type, source_id, account_id
    ) AS cb
        ON cb.source_type = ae.source_type
       AND cb.source_id = ae.source_id
       AND cb.account_id = ae.account_id
    WHERE ae.email != ''
      AND cb.person_id != {{ excluded_person_id() }}
    GROUP BY ae.email
    HAVING uniqExact(cb.person_id) = 1
{%- endif -%}
{% endmacro %}

{#-
  The account-first companion of resolve_person_id(): the CURRENT
  `(source_type, source_id, account_id) -> person_id` map, straight from the
  bindings — no email hop. For facts that carry the author's source account id
  (git pull requests), the account is the source's own primary key for the
  person: it survives an empty or private profile email, a squash-merge that
  unlinks the PR's commits, and an address change. Same claim semantics as the
  email map: the latest `value_type='id'` row per account decides.

  Excluded bindings STAY in this map, unlike the email map's claims: an
  operator binding an account to the excluded person is a statement about the
  account, and it must TERMINATE resolution, not merely decline to help — a
  bot pull request whose commits carry a human's email would otherwise fall
  through to the email map and attribute to that human. Consumers read a
  matched row with person_id = excluded_person_id() as "attribute to nobody,
  and do not consult any other key".

  account_id is normalized lower(trimBoth(...)) on BOTH sides of the join
  (see resolved_person_id_by_account_join): connector identity inputs are not
  uniform about casing, and the fact side must meet whatever the seed stored.

  Same single-resolve-point rule as the email map: resolution semantics live
  here and in resolve_person_id() only.
-#}
{% macro resolve_person_id_by_account() %}
{%- set journal = adapter.get_relation(
        database=target.database, schema='identity', identifier='identity_persons') -%}
{%- if journal is none -%}
    {#- The identity journal has never been created: resolve nothing rather
        than fail the build, mirroring resolve_person_id(). -#}
    SELECT
        ''                                   AS source_type,
        toUUID('00000000-0000-0000-0000-000000000000') AS source_id,
        ''                                   AS account_id,
        toUUID('00000000-0000-0000-0000-000000000000') AS person_id
    WHERE 0
{%- else -%}
    SELECT
        insight_source_type              AS source_type,
        insight_source_id                AS source_id,
        lower(trimBoth(value_effective)) AS account_id,
        person_id
    FROM identity.identity_persons
    WHERE value_type = 'id'
      AND value_effective IS NOT NULL
      AND trimBoth(value_effective) != ''
    ORDER BY
        source_type,
        source_id,
        account_id,
        created_at DESC,
        id DESC
    LIMIT 1 BY source_type, source_id, account_id
{%- endif -%}
{% endmacro %}

{#-
  The join for models whose rows carry (account_source_type,
  account_source_id, account_id). INVARIANT: identity stores
  insight_source_id as sipHash128 of the connector's raw source_id (see the
  connectors' identity_inputs models), while class relations carry the raw
  string — the hash below must stay in lockstep with that minting expression
  or the join silently matches nothing.
-#}
{% macro resolved_person_id_by_account_join(rel) %}
    LEFT JOIN ({{ resolve_person_id_by_account() }}) AS account_map
        ON account_map.source_type = {{ rel }}.account_source_type
       AND account_map.source_id = toUUID(UUIDNumToString(sipHash128(coalesce({{ rel }}.account_source_id, ''))))
       AND account_map.account_id = lower(trimBoth({{ rel }}.account_id))
{% endmacro %}

{#-
  The reserved person meaning "not a human" (bots, CI, service accounts). One
  global constant, unmintable — UUIDv7 never produces an all-ones value — and
  the service binds excluded accounts to it. Every consumer reads it as no
  person; here that means such an account claims no email.
-#}
{% macro excluded_person_id() %}
    toUUID('ffffffff-ffff-ffff-ffff-ffffffffffff')
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
