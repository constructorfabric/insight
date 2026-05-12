---
status: accepted
date: 2026-04-30
decision-makers: roman.mitasov
---

# unique_key formula = `{insight_tenant_id}-{insight_source_id}-{natural_key_parts}`


<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [A. String concat with explicit `-` separator](#a-string-concat-with-explicit---separator)
  - [B. Hash (cityHash64 / MD5) of the same components](#b-hash-cityhash64--md5-of-the-same-components)
  - [C. Per-connector free-form key, no project convention](#c-per-connector-free-form-key-no-project-convention)
  - [D. UUIDv5 from deterministic namespace](#d-uuidv5-from-deterministic-namespace)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-dataflow-adr-unique-key-formula`
## Context and Problem Statement

`cpt-dataflow-principle-rmt-with-version` mandates `ORDER BY (unique_key)` for every dbt-managed table. For that to work as a dedup key globally — i.e. for silver UNION ALL across N connectors not to collide between connectors — every record-producing component (Airbyte connectors, custom Python CDK connectors, dbt explode models, the Rust enrich binary) must compute `unique_key` consistently. We needed an explicit project-wide formula.

## Decision Drivers

- Cross-connector silver UNION ALL must not collide (e.g. Jira's `comment_id=12345` and a hypothetical YouTrack `comment_id=12345` are different things)
- Multi-tenant isolation must be visible in the key (defense-in-depth — even if a tenant filter is missed in WHERE, dedup won't smash tenants together)
- The key should be human-readable in logs / queries / debugging
- The formula must be cheap to compute (no hash function, no UUID generator)
- The formula must be uniformly enforceable across YAML, Python, Rust, and dbt SQL producers

## Considered Options

- **A.** String concatenation with explicit `-` separator: `{insight_tenant_id}-{insight_source_id}-{natural_keys}`
- **B.** Hash (cityHash64 / MD5) of the same components — fixed-size key
- **C.** Per-connector free-form key, no project convention
- **D.** UUIDv5 from a deterministic namespace + record content

## Decision Outcome

Chosen option: **"A. String concat with `-` separator"**, because it is human-readable, trivially uniform across all four producer technologies (YAML AddFields, Python f-strings, Rust `format!`, dbt SQL `concat`), and the natural-key parts are already strings in 99% of upstream APIs (no extra serialization).

The formula:

```
{insight_tenant_id}-{insight_source_id}-{natural_key_part_1}-{natural_key_part_2}-...
```

Tenant first, source second, natural-key last. For SCD2 grains (`class_people`, `cursor__members_snapshot`), the natural-key parts MUST include the version axis (e.g. `valid_from`, `_tracked_at`, `event_id`) so per-version rows get distinct `unique_key`s.

For records produced by dbt explode models (one bronze row → many output rows), the producer MUST compute its own `unique_key` using the same formula — bronze's `unique_key` is at the wrong grain.

For records produced by the Rust enrich binary, the binary MUST compute `unique_key` itself — see `src/ingestion/connectors/task-tracking/jira/enrich/src/io/writer.rs::FieldHistoryInsert::from`.

### Consequences

- Good: human-readable in queries (`SELECT * FROM bronze.foo WHERE unique_key LIKE 'tenant-x-jira-%'`)
- Good: zero compute overhead beyond string concatenation
- Good: same formula in every producer language
- Good: `tenant-source-` prefix makes cross-tenant / cross-instance collisions impossible by construction
- Good: trivial to grep for in code review
- Bad: longer than a hash (storage cost +N bytes/row vs cityHash64 — typically 20-100 bytes)
- Bad: variable length (cityHash64 is fixed UInt64) — slightly less predictable index size
- Bad: connector authors must follow the formula manually — mitigated by the audit / `/check-dbt-conventions` skill / code review

### Confirmation

- `cpt validate` confirms producer code carries `@cpt-*` markers referencing this ADR (audit trail)
- Cypilot skill `/check-dbt-conventions` reads connector configs and asserts the formula structure (LLM-based correctness check)
- Manual audit: existing audit (this session) confirmed 17 of 18 connectors compliant; the one outlier (`claude-admin`) is tracked as follow-up work

## Pros and Cons of the Options

### A. String concat with explicit `-` separator

- Good: human-readable
- Good: trivially uniform across YAML / Python / Rust / SQL
- Good: cheap to compute
- Good: prefix-searchable (`LIKE 'tenant-x-%'`)
- Bad: variable length
- Bad: longer than a hash

### B. Hash (cityHash64 / MD5) of the same components

- Good: fixed-size key (8 bytes for cityHash64)
- Good: smaller index footprint
- Bad: NOT human-readable — debugging requires reverse-mapping
- Bad: hash function call in every producer (each has different hash libraries)
- Bad: collision risk (cosmically small but present)
- Bad: prefix search (`LIKE 'tenant-x-%'`) impossible

### C. Per-connector free-form key, no project convention

- Good: maximum flexibility for connector authors
- Bad: silver UNION ALL across connectors is unsafe (cannot trust collisions don't happen)
- Bad: every reader has to know per-connector key shape
- Bad: no way to validate

### D. UUIDv5 from deterministic namespace

- Good: fixed-size, globally unique
- Good: standard format
- Bad: NOT human-readable
- Bad: requires UUID generation library in every producer (more dependency)
- Bad: harder to debug than option A

## More Information

Implementation across producers:

**YAML connectors** (10 connectors, 40+ streams) — Airbyte declarative `AddFields`:
```yaml
- type: AddFields
  fields:
    - path: [unique_key]
      value: "{{ config['insight_tenant_id'] }}-{{ config['insight_source_id'] }}-{{ record['id'] }}"
```

**Python CDK connectors** (3 connectors, ~20 streams) — helper in stream base class:
```python
def _make_unique_key(tenant_id: str, source_id: str, *natural_key_parts: str) -> str:
    return f"{tenant_id}:{source_id}:{':'.join(natural_key_parts)}"
```
(Note: existing CDK uses `:` separator — flagged as cosmetic deviation, no plan to change yet.)

**dbt explode models** (e.g. `jira__changelog_items`) — SQL `concat`:
```sql
CAST(concat(
    coalesce(insight_source_id, ''), '-',
    coalesce(changelog_id, ''), '-',
    coalesce(field_id, ''), '-',
    coalesce(value_from, ''), '-',
    coalesce(value_to, '')
) AS String) AS unique_key
```

**Rust enrich** (`writer.rs::FieldHistoryInsert::from`):
```rust
let unique_key = format!(
    "{}-{}-{}-{}-{}",
    r.insight_source_id, data_source, r.id_readable, r.field_id, r.event_id
);
```

**dbt staging models** (most cases) — pass-through from bronze:
```sql
SELECT
    u.unique_key AS unique_key,
    ...
FROM {{ source('bronze_<connector>', '<table>') }} u
```

**SCD2 extension** (`to_class_people.sql`):
```sql
CAST(concat(coalesce(unique_key, ''), '-', toString(lastChanged)) AS String) AS unique_key
```

## Traceability

- **DESIGN**: [DESIGN.md](../DESIGN.md)
- **Sibling ADRs**:
  - `cpt-dataflow-adr-rmt-with-version-and-unique-key` — establishes WHY a single dedup column is needed
  - `cpt-dataflow-adr-promote-bronze-to-rmt` — uses `unique_key` as bronze ORDER BY
  - `cpt-dataflow-adr-ephemeral-rust-passthrough` — defines how Rust producers must implement this formula

This decision directly addresses the following design elements:

* `cpt-dataflow-principle-unique-key-formula` — mandates the formula
* `cpt-dataflow-component-bronze` — connectors compute `unique_key` per this formula on insertion
* `cpt-dataflow-component-staging` — staging models propagate or compute `unique_key` per this formula
* `cpt-dataflow-component-rust-enrich` — Rust binary computes `unique_key` per this formula
