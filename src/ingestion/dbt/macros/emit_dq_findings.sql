{#-
  emit_dq_findings()
  ---------------------------------------------------------------------------
  Project-level `on-run-end` hook. After any dbt invocation it walks `results`
  and emits ONE structured JSON line per *test* node — a "data-quality
  finding" — to stdout. A log collector ships these lines to the central log
  store, where they are queried and alerted on.

  This is the single emission point for the whole check catalog. Every test
  flows through here, silver and gold alike (gold checks are ordinary singular
  tests that read the `insight` views via {{ source('gold', ...) }}). dbt
  supplies status + failure count + timing from the run result and severity +
  meta + compiled SQL from the node, so there is no second catalog to maintain.

  Finding shape:
    event           always "data_quality_finding"
    run_id          dbt invocation_id (one run = one batch of findings)
    check_id        test node name (convention: assert_<subject>_<rule>)
    title           human label (meta.title, falls back to check_id)
    domain          meta.domain  (collab | git | task | ai | hr | gold | ...)
    category        meta.category (source_uniqueness | physical_bound | grain | freshness | ...)
    gate            dbt severity (warn | error) — whether a violation is advisory or blocking
    tier            meta.tier (info | warn | error) — triage importance to a human
    status          run outcome (pass | warn | fail | error | skipped)
    rows_violating  failing-row count from the test result
    duration_ms     test execution time
    audit_relation  store_failures table holding the violating rows (null when not stored)
    remediation     meta.remediation — operator fix hint

  Low-cardinality fields (domain / category / tier / status / check_id) are
  meant to become log labels; everything else is the JSON body. Sample rows are
  NOT emitted here — they live in the store_failures table that
  `audit_relation` points at, fetched on demand.

  The hook runs on every invocation but only emits for test nodes, so a plain
  `dbt run` (models only) emits nothing.
-#}
{% macro emit_dq_findings() %}
  {%- if execute and results -%}
    {%- for r in results if r.node.resource_type == 'test' and 'data_quality' in r.node.tags -%}
      {%- set meta = r.node.config.meta or {} -%}
      {%- set finding = {
          'event': 'data_quality_finding',
          'run_id': invocation_id,
          'check_id': r.node.name,
          'title': meta.get('title', r.node.name),
          'domain': meta.get('domain', 'unknown'),
          'category': meta.get('category', 'uncategorized'),
          'gate': (r.node.config.severity | string),
          'tier': meta.get('tier', 'warn'),
          'status': (r.status | string),
          'rows_violating': (r.failures if r.failures is not none else 0),
          'duration_ms': ((r.execution_time | float) * 1000) | round | int,
          'audit_relation': r.node.relation_name,
          'remediation': meta.get('remediation', none)
      } -%}
      {#- print() writes the raw line to stdout (no dbt timestamp/level prefix),
          so a log collector can parse `DQ_FINDING {json}` directly. -#}
      {%- do print('DQ_FINDING ' ~ (finding | tojson)) -%}
    {%- endfor -%}
  {%- endif -%}
{% endmacro %}
