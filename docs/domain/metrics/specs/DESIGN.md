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
- `observed_at` is reserved for future point-in-time semantics.
- `subject_key` carries the counted subject for distinct-count measures (a
  date, a tool) and is NULL on every other measure's rows.
- A row is emitted only when the source provides a value; `value` is never
  NULL (the column stays nullable in the contract).
- Dimension values and labels come from class-contract columns declared by
  staging models; gold does not synthesize fallbacks.
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

Observation relation names are data, not code: `metric_sources.source_ref`
stores the relation name, constrained to the `<family>_metric_observations`
naming shape (lowercase `snake_case`, `insight` database) and parsed on
every load. A relation becomes queryable only after the schema validator
probes its columns against `OBSERVATION_COLUMNS`. Adding an observation
source is therefore a dbt gold model plus registry seed rows — no backend
enum or table-name code change. All observation relations share one column
contract; a source that needs different columns is a different source kind.

Gold models are built at deploy time by the ClickHouse migrate hook
(`dbt run --select tag:gold`, final step of
`src/ingestion/scripts/apply-ch-migrations.sh`), so the views exist before
any connector sync — bronze/silver placeholders guarantee the DDL
type-checks on a fresh cluster. Per-connector scoped dbt runs keep them
current afterwards.

The cohort view is unique per `(tenant_id, entity_type, entity_id,
cohort_key)`. The peer query relies on this; a dbt build-integrity test
asserts it.

## Computations

The computation vocabulary is closed and fully executable:

```text
sum
ratio
median
distinct_count
```

Semantics:

- `sum`: sum one numeric measure.
- `ratio`: aggregate numerator and denominator measures first, then divide.
- `median`: exact middle (`quantileExact(0.5)`) of per-event observation
  values. No `scale`. Median measures emit one row per source event via the
  `event_measure` shape macro; multiple rows per (entity, day, measure) are
  the intended grain. A median over no rows is NULL — medians are never
  zero-filled.
- `distinct_count`: exact count of distinct `subject_key` values
  (`uniqExact`) over the entity's observations — distinct active dates,
  distinct tools. No `scale`. Distinct-count measures emit one row per
  subject via the `distinct_measure` shape macro, stamping the subject on
  `subject_key`; `value` carries a constant 1 so the same measure can also
  serve as a sum-computation row count (e.g. a ratio denominator). Zero
  distinct subjects is a genuine zero — distinct counts zero-fill like sums.

Ratios use:

```text
sum(numerator) / nullIf(sum(denominator), 0) * scale
```

They are not averages of row-level ratios. A ratio whose numerator measure
has no rows at all is NULL, not zero: a source that reports the denominator
but never the numerator (a chat tool with totals but no message-type split)
has not measured the split, and rendering it as 0% would fabricate an
observation.

Ratio numerator and denominator inputs must resolve to measures of the same
source. Cross-source ratios are a configuration error.

Row granularity is a property of the measure's shape macro: `sum_measure`
and `presence_measure` emit day-aggregated rows; `event_measure` emits one
row per source event for median inputs; `distinct_measure` emits one row
per counted subject for distinct-count inputs. Binding a measure to a
computation whose grain it does not carry is a configuration error in the
registry review, not detectable at runtime.

Extending the vocabulary (anticipated kinds: further distribution
statistics, point-in-time gauges over `observed_at`, derived expressions
over other metrics) is one coordinated change: a `ComputationSpec` variant,
a compiler arm, the `computation_type` DB enum, a shape macro if the
observation shape is new, and the response `computation` tag. Nothing is
stored before it executes.

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
explanation
unit
format
direction
entity_type
computation_type
scale
peer_cohort_key
origin
is_enabled
schema_status
schema_error_code
```

`unit` is a display suffix for formats that do not fully determine
presentation on their own (e.g. `"lines"`, `"days"`, `"h"`). `percent` and
`currency` are presentation-complete — the frontend renders `%` or a
currency symbol from `format` alone and never consults `unit` for these two
formats — so `unit` must be `None` for any metric with one of those two
formats. Pinned by a builtin registry test
(`presentation_complete_formats_carry_no_unit`); a future format-as-union
refactor (folding unit into format-specific variants) would make this
invalid by construction, but is not warranted while the registry test
enforces it and only builtins populate the table.

`metric_definition_inputs` maps input roles to source measures:

```text
value
numerator
denominator
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
      | { view: "histogram" }
    >
  }>
}
```

Response:

```ts
type MetricResult = {
  metric_key: string
  label: string
  description?: string
  explanation?: string
  unit: string | null
  format: "integer" | "decimal" | "currency" | "percent"
  direction: "higher_is_better" | "lower_is_better" | "neutral"
  views: MetricResultView[]
} & (
  | { computation: "sum" }
  | { computation: "ratio"; scale: number }
  | { computation: "median" }
)
```

The computation tag and its fields are flattened into the result object; a
serde wire-shape test in `metric_results/builder.rs` pins this layout.

The histogram view shape:

```ts
{ view: "histogram"; values: Array<{ entity_id: string; bins: Array<{ lo: number; hi: number; count: number }> }> }
```

View values use `entity_id`, not person-specific fields.

## Runtime Flow

1. Resolve tenant from request context.
2. Validate entity, period, metric keys, view specs, and dimensions.
3. Load visible metric definitions from DB.
4. Convert DB rows into Rust discriminated unions.
5. Compile one ClickHouse query per requested metric view.
6. Execute queries with bounded concurrency.
7. Shape rows into typed result views.
8. Enforce final response row cap.
9. Return metrics in request order.

Execution rules:

- `sum` no rows returns `0`.
- `ratio` missing or zero denominator returns `null`.
- `median` no rows returns `null` — medians are never zero-filled.
- Histograms are valid only for `median` metrics: they bin per-event
  observation values into 10 server-owned fixed-width bins over the
  entity's own exact `[min, max]`; the last bin is closed at the maximum,
  all-identical values collapse to a single `[v, v]` bin, and an entity
  with no events is listed with an empty `bins` array. Binning is
  deterministic arithmetic over exact aggregates — never the adaptive
  `histogram()` aggregate.
- Ungrouped timeseries are dense per requested entity and bucket.
- Dimensioned timeseries are dense per requested entity, observed dimension group, and bucket.
- Rows missing a requested dimension group under value `__unknown__` with
  label `Unknown` (runtime guard; the schema validator's coverage probe makes
  this rare).
- Breakdown returns observed dimension groups only.
- The cohort view scopes who counts as a peer; only members with observed
  values contribute to the percentiles. The peer query never fabricates zero
  observations: absence of rows is indistinguishable from "not covered by the
  source" (no seat, no account), so inventing zeros would rank people the
  data never measured. A source for which covered-but-inactive genuinely
  means zero can emit explicit zero observations — the coverage knowledge
  lives in the connector, not the runtime.
- Peer measurability is therefore an emission decision each gold view makes
  per measure, and it has exactly two defensible gates. Value-gated
  emission (a row whenever the source reports the person, zeros included)
  puts measured zeros in peer pools — right when zero is a behavioral
  outcome of an engaged person (a quiet email week, a calendar with
  meetings every day). Engagement-gated emission (rows only on deliberate
  activity) keeps pools to engaged users — right when zero means
  non-engagement (rostered but absent accounts), which would otherwise drag
  medians toward zero and rank people who are not participating. Activity
  metrics (active days, distinct tools) are engagement-gated; volume and
  outcome metrics are value-gated. Changing a measure's gate re-ranks every
  peer standing on that metric: it must be an explicit decision, never a
  side effect of a connector reshaping what it emits.
- Target entities missing cohort membership are omitted from peer values.
- Target entities without observed values get a null `target_value`.
- Null values are excluded from peer percentiles and `n`.
- Peer percentiles and min/max are suppressed (returned as null) when the
  peer pool has fewer than 5 distinct observed members; `n` reports that
  distinct count. Quartiles over a handful of people are noise, and tiny
  pools disclose individual values. Enforced server-side so every consumer
  inherits it, and counted with `uniqExact` so duplicate cohort membership
  rows can neither inflate the pool nor defeat the floor.

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
- a histogram view targets a non-median metric.
- projected or final result size exceeds the row cap (histogram views
  project `entities × 10` rows).

## Authorization

v1 decision: any authenticated member of a tenant may query metric results
for any entity ids in that tenant. Peer views expose aggregates only (no peer
entity ids); period, timeseries, and breakdown views expose per-entity values.
Entity-level scoping (self, reports, role-based) is deferred to the real
authorization system; this endpoint must adopt it when it lands.

Warehouse tenant isolation is not implemented platform-wide: compiled queries
do not filter on the warehouse `tenant_id` column, matching the rest of the
platform's single-tenant posture. The control-plane tenant id has no defined
mapping to the warehouse `tenant_id` strings stamped at ingestion; defining
that mapping and adding the predicate (one place: the compiler's shared WHERE
clause) is the multi-tenant unlock. The observation and cohort contracts keep
the column so that change needs no contract migration.

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
- the validator sweeps periodically, not once at startup: managed relations
  are dbt-created and may appear after the service boots (fresh deploys) or
  regress later (a bad model change); both converge within one sweep with no
  restart.
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
   relations)` for row-existence markers, `event_measure(measure_key,
   relation, value_expr, dimensions_col, where=none)` for per-event values
   feeding median metrics. Every branch is a shape-macro call; a new macro
   is added only when a new computation kind becomes executable.
   Read only class-contract columns; never vendor-specific ones — if the fact
   you need is not in the class contract, extend the class contract first
   (staging models declare semantics, see the class `schema.yml`).
2. Add the `measure_key` to the gold model's `schema.yml` `accepted_values`
   test.
3. Add the measure key to the source's `measures` list in `builtin.rs`.
4. Add the `MetricSeed` as in case 1.
5. Validate: `dbt parse` + `cargo test -p analytics` (see Validation
   commands).

### Case 3: new observation source

The metric family reads data no managed source covers.

1. Create a dbt gold model in `src/ingestion/gold/` named
   `<family>_metric_observations`, emitting the source measure observation
   contract, `schema=insight`, `ref()`-ing silver models (medallion layering
   rules: `docs/domain/ingestion-data-flow/specs/DESIGN.md`). Document columns
   and measure keys in `src/ingestion/gold/schema.yml`.
2. Add a `BuiltinSource` (source + measures + dimensions) to `builtin.rs`,
   with `source_ref` set to the relation name. No backend enum or table-name
   code changes: the relation name is data, validated on load against the
   `<family>_metric_observations` shape (`ObservationRelation`) and probed at
   runtime by the schema validator.
3. Add `MetricSeed`s as in case 1.
4. Validate: `dbt parse` + `cargo test -p analytics` (see Validation
   commands). The runtime schema validator probes the new relation at
   startup.

### Rules that hold for every case

- No metric-key-specific branches in runtime code.
- No vendor names, vendor columns, or label mappings in gold models — labels
  and taxonomy come from class-contract columns declared by staging.
- Measure filter predicates (`where=` on shape macros) may reference only
  class-contract dimension columns and their normalized values — never vendor
  columns, tool names, or label text.
- Adding a class-contract column that a gold model reads needs TWO things,
  because the gold model is built at deploy time — before any connector
  re-syncs — against class tables that already exist as real (non-placeholder)
  relations:
  1. Schema presence at deploy: add the column unconditionally via a
     ClickHouse migration (`ADD COLUMN IF NOT EXISTS`). The placeholder script
     only reconciles placeholder-marked tables, so it does NOT cover existing
     installs; without the migration the gold `dbt run` fails on the missing
     column and `--atomic` rolls the whole upgrade back.
  2. Values: columns derived from source data require re-materialization —
     major-bump every affected connector (ADR-0015 dispatches a scoped
     one-shot full refresh; CDK connectors need an explicit invocation until
     the toolkit closes its semver-storage gap), and stay NULL until it runs.
     Declared-constant columns (labels) are backfilled in place by the same
     migration; any rebuild independently converges to the same values.
- No new `metric_catalog` seed migrations and no new ad-hoc `insight.*` views
  for metrics.
- Do not add runtime formula JSON until generation exists.

### Validation commands

```sh
# from src/backend — registry invariants, enum round-trips, compiler tests
cargo test -p analytics

# from src/ingestion/dbt — manifest validation, no warehouse connection.
# CI runs the same gate (build-images.yml, toolbox job).
dbt parse --profiles-dir <dir-with-dummy-profile>
```

The dummy profile is a `profiles.yml` with profile name `ingestion` and any
unreachable `type: clickhouse` output; `dbt parse` loads the adapter but never
connects.

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
- renders using returned label, description, explanation, unit, format,
  direction, and computation.
- owns chart choice and layout.

Backend responses do not include chart metadata.

## Non-Goals

- No custom metric authoring UI in this pass.
- No custom SQL execution in this pass.
- No public source labels in metric results.
- No metric-key-specific branches in result compilation.
- No partial responses for oversized results.
