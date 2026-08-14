{#-
  Creates `identity.identity_persons` — the persons-log copy that dbt does NOT
  own the DATA of. It is written exclusively by the identity-resolution
  service's persons-sync (full snapshot + atomic EXCHANGE swap); gold models
  LEFT JOIN it through the `resolve_person_id()` macro to attach a canonical
  `person_id` to email-keyed observations.

  Called from `on-run-start` (same pattern as `create_task_field_history_staging`)
  so a build on an environment where the sync has never run — fresh cluster,
  CI, local k3d — meets an EMPTY table instead of a missing one: every
  resolve comes back NULL and the pipeline behaves exactly as before the
  person_id column existed. Graceful degradation, not a build failure.

  The resolver's other input, `identity.identity_inputs`, is deliberately NOT
  created here: dbt owns that relation (an incremental silver model). Creating
  it early would make its first run think it is incremental and filter its own
  seed rows away. The resolver degrades on its own when the relation is absent.

  SCHEMA CONTRACT: this DDL is a byte-for-byte copy of COLUMNS_DDL in
  src/backend/services/identity-resolution/src/infra/identity_persons.rs —
  the service owns the schema (it mirrors its own MariaDB `persons` log,
  `001_persons.sql`, minus the generated `value_hash`, plus the `_synced_at`
  watermark). If the service changes the schema, change THIS macro in the
  same PR. A drifted hook is mostly harmless (CREATE IF NOT EXISTS never
  alters an existing table; the service's own staging-swap upgrades the live
  schema on its next run) but a fresh environment would create the stale
  shape — keep them in lockstep.
-#}

{% macro create_identity_persons() %}
    {% do run_query("CREATE DATABASE IF NOT EXISTS identity") %}

    {% do run_query("
        CREATE TABLE IF NOT EXISTS identity.identity_persons
        (
            id                  UInt64,
            value_type          String,
            insight_source_type String,
            insight_source_id   UUID,
            insight_tenant_id   UUID,
            value_id            Nullable(String),
            value_full_text     Nullable(String),
            value               Nullable(String),
            value_effective     Nullable(String),
            person_id           UUID,
            author_person_id    UUID,
            reason              Nullable(String),
            created_at          DateTime64(6, 'UTC'),
            _synced_at          DateTime64(3, 'UTC')
        )
        ENGINE = MergeTree
        ORDER BY id
    ") %}
{% endmacro %}
