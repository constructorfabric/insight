# Technical Design — Metrics

Status: active implementation contract.

The metrics system computes metric result views from typed metric definitions
and normalized source measure observations. Metrics are authored, requested,
and rendered through one structured path: a registry defines metric semantics,
dbt gold models emit source measure observations, and one generic runtime
compiles and serves every metric.

New metrics MUST be added through this system. The legacy path — ad-hoc
`insight.*` gold views in `src/ingestion/scripts/migrations/` plus
`metric_catalog` seed migrations — is frozen for new metrics and remains only
until its existing consumers migrate.

## Goals

- Define metrics once and query them through one generic runtime.
- Model metrics by semantic computation, not by current UI cards.
- Support multiple entity types, with `person` as the first consumer.
- Keep backend responses self-describing enough for frontend rendering.
- Keep chart choice and layout out of backend metric contracts.
- Use typed Rust and TypeScript unions for states that cannot coexist.

## Source Measure Observation Contract

Managed observation sources expose rows shaped like:

```sql
tenant_id String,
source_key String,
entity_type String,
entity_id String,
metric_date Date,
observed_at Nullable(DateTime64(3)),
measure_key String,
value Nullable(Float64),
subject_key Nullable(String),
dimensions Array(Tuple(key String, value String, label Nullable(String)))
```

Rules:

- Observations belong to source measures, not final metrics.
- `source_key` identifies the logical source.
- `measure_key` identifies the source measure.
- `entity_type` and `entity_id` identify the measured entity.
- `observed_at` is available for gauge/latest semantics.
- `subject_key` is available for distinct-count semantics.
- Missing dimensions use value `__unknown__` and label `Unknown`.
- Observations do not contain chart metadata.
- Observations do not contain cohort membership. Peer comparison reads the
  cohort view directly.

## Managed Source Ownership

Managed observation sources and the cohort view are dbt gold models
(`src/ingestion/gold/`), materialized as views in the `insight` database:

- `insight.ai_metric_observations`
- `insight.metric_entity_cohorts_current`

dbt owns lineage to silver, build ordering, column documentation, and data
tests (including cohort uniqueness). The backend owns the registry, query
compilation, and runtime schema validation against these relations. Column
changes are coordinated changes: dbt model + `schema.yml` + backend
`OBSERVATION_COLUMNS`/`COHORT_COLUMNS` + this document.

The cohort view is unique per `(tenant_id, entity_type, entity_id,
cohort_key)`. The peer query relies on this; a dbt build-integrity test
asserts it.

## Computations

Executable computation vocabulary:

```text
sum
count
count_distinct
ratio
distribution
gauge
derived
```

Current execution support:

```text
sum      executable
ratio    executable
others   typed and stored, rejected with UNSUPPORTED_COMPUTATION when requested
```

Semantics:

- `sum`: sum one numeric measure.
- `count`: count source rows for one event measure.
- `count_distinct`: count distinct `subject_key` values.
- `ratio`: aggregate numerator and denominator measures first, then divide.
- `distribution`: compute one configured statistic from sample values.
- `gauge`: compute one configured snapshot method.
- `derived`: reserved for expressions over other metrics.

Ratios use:

```text
sum(numerator) / nullIf(sum(denominator), 0) * scale
```

They are not averages of row-level ratios.

Ratio numerator and denominator inputs must resolve to measures of the same
source. Cross-source ratios are a configuration error; they belong to the
`derived` computation when it becomes executable.

## Storage Model

Metric definitions are stored separately from legacy metric/catalog concepts.

Tables:

```text
metric_sources
metric_source_measures
metric_source_dimensions
metric_definitions
metric_definition_inputs
metric_definition_dimensions
```

`metric_sources` stores typed source refs.

`metric_source_measures` stores measures available from a source.

`metric_source_dimensions` stores dimensions available from a source.

`metric_definitions` stores metric metadata and computation type:

```text
metric_key
label
description
unit
format
direction
entity_type
computation_type
scale
distribution_statistic
gauge_method
peer_cohort_key
origin
definition_version
is_enabled
schema_status
schema_error_code
```

`metric_definition_inputs` maps input roles to source measures:

```text
value
event
numerator
denominator
sample
snapshot
dependency
```

`metric_definition_dimensions` maps metrics to source dimensions.

Rules:

- Product definitions have `tenant_id = NULL`.
- Tenant definitions override product definitions for the same key.
- Disabled definitions, sources, or measures are unavailable.
- Schema-error definitions, sources, or measures are unavailable.
- A disabled or schema-error tenant definition falls back to the product
  definition for the same key instead of shadowing it.
- Raw DB source refs are converted into typed backend enums before SQL generation.

## Builtin Seed Reconciliation

Builtin definitions are declared in one code registry
(`src/backend/services/analytics/src/domain/metric_definitions/builtin.rs`)
and converged into the DB by a startup reconciler, not by migrations.
Migrations own schema only.

Rules:

- The reconciler runs synchronously after migrations, before serving traffic,
  and on the `migrate` CLI command.
- Upserts are idempotent and race-safe across replicas.
- Builtin sources, measures, and definitions absent from the registry are
  disabled, never deleted.
- Source dimension rows have no enabled flag; rows removed from the registry
  stay in place and are inert unless a definition links them.
- Tenant-owned rows are never touched by reconciliation.
- Warm environments converge to the registry state on every deploy.

## Result API

Endpoint:

```http
POST /v1/metric-results
```

Request:

```ts
type MetricResultsRequest = {
  entity: { type: string; ids: string[] }
  period: { from: string; to: string }
  metrics: Array<{
    metric_key: string
    views: Array<
      | { view: "period" }
      | { view: "peer"; cohort_key?: string }
      | { view: "timeseries"; bucket?: "day" | "week" | "month"; dimensions?: string[] }
      | { view: "breakdown"; dimensions: string[] }
    >
  }>
}
```

Response:

```ts
type MetricResult =
  | { computation: "sum"; views: MetricResultView[] }
  | { computation: "count"; views: MetricResultView[] }
  | { computation: "count_distinct"; views: MetricResultView[] }
  | { computation: "ratio"; scale: number; views: MetricResultView[] }
  | { computation: "distribution"; statistic: string; views: MetricResultView[] }
  | { computation: "gauge"; method: string; views: MetricResultView[] }
  | { computation: "derived"; views: MetricResultView[] }
```

Every metric result also includes:

```text
metric_key
label
description
unit
format
direction
```

View values use `entity_id`, not person-specific fields.

## Runtime Flow

1. Resolve tenant from request context.
2. Validate entity, period, metric keys, view specs, and dimensions.
3. Load visible metric definitions from DB.
4. Convert DB rows into Rust discriminated unions.
5. Reject unsupported computations with `UNSUPPORTED_COMPUTATION`.
6. Compile one ClickHouse query per requested metric view.
7. Execute queries with bounded concurrency.
8. Shape rows into typed result views.
9. Enforce final response row cap.
10. Return metrics in request order.

Execution rules:

- `sum` no rows returns `0`.
- `ratio` missing or zero denominator returns `null`.
- Ungrouped timeseries are dense per requested entity and bucket.
- Dimensioned timeseries are dense per requested entity, observed dimension group, and bucket.
- Breakdown returns observed dimension groups only.
- Peer starts from the generic current cohort view so zero-activity peers can be included.
- Target entities missing cohort membership are omitted from peer values.
- Null values are excluded from peer percentiles and `n`.

## Validation

Request caps, checked before any per-request enumeration work:

- at most 50 metrics per request.
- at most 1000 entity ids per request.
- at most 400 days per period.

Entity id normalization is a property of the entity type: `person` ids are
emails and are trimmed and lowercased to match observation sources, which
emit lowercased emails; other entity types are trimmed only.

Reject with a client error when:

- entity type or ids are empty.
- a request cap is exceeded.
- period dates are invalid or reversed.
- metrics are empty.
- metric keys are empty, duplicated, unknown, disabled, or schema-error.
- a metric requests no views.
- a metric requests the same view twice.
- a requested dimension is empty, duplicated, or not declared for the metric.
- a breakdown has no dimensions.
- a peer view has no requested or default cohort key.
- projected or final result size exceeds the row cap.
- computation is typed but not executable.

## Authorization

v1 decision: any authenticated member of a tenant may query metric results
for any entity ids in that tenant. Peer views expose aggregates only (no peer
entity ids); period, timeseries, and breakdown views expose per-entity values.
Entity-level scoping (self, reports, role-based) is deferred to the real
authorization system; this endpoint must adopt it when it lands.

Schema validation checks:

- managed source refs map to backend source enums.
- source observation views expose required columns.
- generic cohort view exposes required columns.
- declared dimensions are present on every recent row of each observed input
  measure; a covered-measure gap is a schema error.
- input measures without recent observations downgrade the definition to
  `unchecked`, never `error`: filtered measures legitimately go quiet, and
  absence of data is indistinguishable from an unemitted measure.
- probe failures never overwrite a previously established status.
- warehouse diagnostics stay server-side.

## Adding a Metric

Built-in metrics are authored by Insight developers through the registry and
the managed observation models. There are exactly three cases; pick the first
one that applies.

### Case 1: metric over an existing measure

The measure already appears in a managed observation source (check the
`measures` list of the source in `builtin.rs` and the emitting gold model).

1. Add one `MetricSeed` to `BUILTIN_METRICS` in
   `src/backend/services/analytics/src/domain/metric_definitions/builtin.rs`:
   metric key (`namespace.metric_name`, lowercase snake case), label,
   description, unit, format, direction, entity type, computation type,
   input role mapping to the measure, allowed dimensions, peer cohort key.
2. Run `cargo test -p analytics` — the registry invariant tests validate
   key shapes, input/measure references, and computation field combinations.

The reconciler seeds the definition on the next deploy. No SQL, no migration,
no dbt change.

### Case 2: new measure from an existing source

The source exists but does not emit the measure yet.

1. Add the measure branch to the source's gold model in `src/ingestion/gold/`:
   one `UNION ALL` entry calling a shape macro from
   `src/ingestion/dbt/macros/metric_observation_measures.sql` —
   `sum_measure(measure_key, relation, value_expr, dimensions_col,
   where=none)` for aggregated numerics, `presence_measure(measure_key,
   relations)` for row-existence markers. Every branch is a shape-macro call;
   a new macro is added only when a new computation kind becomes executable.
   Read only class-contract columns; never vendor-specific ones — if the fact
   you need is not in the class contract, extend the class contract first
   (staging models declare semantics, see the class `schema.yml`).
2. Add the `measure_key` to the gold model's `schema.yml` `accepted_values`
   test.
3. Add a `MeasureSeed` to the source in `builtin.rs`.
4. Add the `MetricSeed` as in case 1.
5. Validate: `dbt parse` (dummy profile) + `cargo test -p analytics`.

### Case 3: new observation source

The metric family reads data no managed source covers.

1. Create a dbt gold model in `src/ingestion/gold/` emitting the source
   measure observation contract, `schema=insight`, `ref()`-ing silver models.
   Document columns and measure keys in `src/ingestion/gold/schema.yml`.
2. Add an `ObservationSource` enum variant and `from_ref`/`table_ref` mapping
   in `src/backend/services/analytics/src/domain/metric_definitions/definition.rs`.
3. Add a `BuiltinSource` (source + measures + dimensions) to `builtin.rs`.
4. Add `MetricSeed`s as in case 1.
5. Validate: `dbt parse` + `cargo test -p analytics`. The runtime schema
   validator probes the new relation at startup.

### Rules that hold for every case

- No metric-key-specific branches in runtime code.
- No vendor names, vendor columns, or label mappings in gold models — labels
  and taxonomy come from class-contract columns declared by staging.
- Measure filter predicates (`where=` on shape macros) may reference only
  class-contract dimension columns and their normalized values — never vendor
  columns, tool names, or label text.
- No new `metric_catalog` seed migrations and no new ad-hoc `insight.*` views
  for metrics.
- Do not add runtime formula JSON until generation exists.

Future developer-side generation may use source models and formulas to produce
the managed observation SQL and seed rows, but runtime execution still
consumes typed definitions and source measure observations.

## Custom Metric Gate

Runtime-authored metrics require one of:

- generated managed observation SQL plus generated definition/source seed rows.
- validated custom observation SQL that emits the source measure observation contract.

Until one exists, custom definitions can be stored but cannot produce new source observations. The runtime only executes metrics whose inputs resolve to available, validated source measures.

## Frontend Contract

Frontend collection rendering:

- requests metric keys and views.
- treats configured required views as required.
- normalizes response arrays only for local lookup.
- renders using returned label, unit, format, direction, and computation.
- owns chart choice and layout.

Backend responses do not include chart metadata.

## Non-Goals

- No custom metric authoring UI in this pass.
- No custom SQL execution in this pass.
- No public source labels in metric results.
- No metric-key-specific branches in result compilation.
- No partial responses for oversized results.
