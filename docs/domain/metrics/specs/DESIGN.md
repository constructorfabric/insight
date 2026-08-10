# Technical Design — Metrics


<!-- toc -->

- [Goals](#goals)
- [Source Measure Observation Contract](#source-measure-observation-contract)
- [Managed Source Ownership](#managed-source-ownership)
- [Metric Evidence Contract](#metric-evidence-contract)
- [Computations](#computations)
- [Storage Model](#storage-model)
- [Builtin Seed Reconciliation](#builtin-seed-reconciliation)
- [Result API](#result-api)
- [Runtime Flow](#runtime-flow)
- [Validation](#validation)
- [Authorization](#authorization)
- [Adding a Metric](#adding-a-metric)
  - [Case 1: metric over an existing measure](#case-1-metric-over-an-existing-measure)
  - [Case 2: new measure from an existing source](#case-2-new-measure-from-an-existing-source)
  - [Case 3: new observation source](#case-3-new-observation-source)
  - [Rules that hold for every case](#rules-that-hold-for-every-case)
  - [Validation commands](#validation-commands)
- [Custom Metrics](#custom-metrics)
  - [Execution wire](#execution-wire)
  - [Observation contract the custom SQL must emit](#observation-contract-the-custom-sql-must-emit)
  - [Reconcile-safety of custom rows](#reconcile-safety-of-custom-rows)
  - [Validation and execution role](#validation-and-execution-role)
- [Frontend Contract](#frontend-contract)
- [Non-Goals](#non-goals)

<!-- /toc -->

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
- `entity_type` and `entity_id` identify the measured entity — one
  polymorphic pair, on the wire and in the relations alike.
- For `entity_type = 'person'` that identity IS the canonical person id,
  resolved from the identity log at build time (`resolve_person_id` dbt
  macro). Gold therefore serves canonical grain: one row per person, with a
  person's several source emails already summed together. A source row whose
  email identity cannot resolve is ABSENT rather than guessed — with
  `entity_id` being the person id there is nothing to serve it under.
  Unresolved work is not lost from view: the evidence relations keep every
  source row keyed by its source-native email, and
  `insight.identity_resolution_coverage` reports the gap per source from
  there.
- The cohort relation is canonical-grained under the same rule, and a person
  whose HR emails claim different org units is EXCLUDED from peer comparison
  (contested membership is never tie-broken). The peer query reads that grain
  straight; it does not repair it per request.
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
(`src/ingestion/gold/`), materialized as views or MergeTree serving tables in
the `insight` database:

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
`src/ingestion/scripts/apply-ch-migrations.sh`), so the relations exist before
any connector sync — bronze/silver placeholders guarantee the DDL
type-checks on a fresh cluster. Per-connector scoped dbt runs keep them
current afterwards.

The cohort view is unique per `(tenant_id, entity_type, entity_id,
cohort_key)`. The peer query relies on this; a dbt build-integrity test
asserts it.

## Metric Evidence Contract

Each managed source may expose `<family>_metric_evidence` in the `insight`
database:

```sql
tenant_id String,
source_key String,
entity_type String,
entity_id String,
metric_date Date,
observed_at Nullable(DateTime64(3)),
measure_key String,
record_id String,
record_kind String,
granularity String,
record_label String,
contribution Nullable(Float64),
subject_key Nullable(String),
dimensions Array(Tuple(key String, value String, label Nullable(String))),
details Map(String, String)
```

Evidence relations are MergeTree serving tables built from silver. Their
ordering follows the drilldown predicate and cursor access pattern, avoiding
repeated silver reconstruction for every page and export. Observation models
derive their values from these evidence tables. The registry stores one
evidence relation per source and one granularity per measure:

- `event`: one source event, such as a commit.
- `source_summary`: the finest summary preserved by silver.
- `derived_population`: a source entity participating in a derived metric.

All managed evidence and observation tables use the shared
`metric_evidence_table` and `metric_observations_table` dbt macros. They own
materialization, storage keys, partitioning, tags, and bounded query settings.
These settings apply uniformly; model-specific query settings are retained
only when required by model semantics.

The tables are partitioned by calendar month from `metric_date`. Evidence is
ordered by tenant, source, entity type, entity, measure, date, and record ID;
observations omit the final record ID. Monthly partitioning is physical
storage only: it does not change metric dates, timestamps, row granularity, or
drilldown results. Each insert block may address at most 512 partitions.

dbt builds a replacement table and exchanges it only after the build
succeeds. A failed or cancelled build therefore leaves the active table in
place, and the next build removes abandoned temporary relations. Replacement
is atomic per table, not across the complete gold DAG. Replacing evidence
invalidates active evidence cursors through the existing snapshot-expired
contract.

Definitions do not declare a separate drilldown strategy. The runtime resolves
the definition's existing input roles and source measures, requires every input
to use the same evidence relation, and compiles the evidence selection from
that metadata. A new metric over existing evidence-backed measures therefore
inherits drilldown without metric-specific SQL, backend branches, or frontend
configuration.

The schema validator probes every standard column. Drilldown capability is
absent until the probe is definitively healthy and every metric input has
granularity metadata. Missing, unchecked, or invalid evidence fails closed.
`POST /v1/metric-results` and `GET /v1/metric-definitions` expose that
capability; consumers omit evidence actions when it is absent.

The evidence runtime owns presentation. It projects the internal contract into
typed human-facing columns rather than exposing `record_kind`, input role,
dimensions, or other storage fields directly:

- source-summary and derived-population measures default to date plus value.
- event measures declare reusable detail keys by `(source_key, measure_key)`,
  such as ref, title, repository, author, or issue type.
- selected chart dimensions are added from the typed `dimensions` array.
- ratio metrics return daily columns named after their numerator and
  denominator measures instead of an ambiguous value column.
- unknown detail keys are humanized and treated as strings; fields requiring
  another label or type are added to the centralized presentation registry.

Presentation is source-measure metadata in runtime code today. It is declared
once per reusable measure shape, never once per metric and never in frontend
configuration.

`POST /v1/metric-drilldown` accepts one metric, one person entity, a period,
declared dimension filters, and an encoded continuation cursor. It returns the
canonical selection, typed server-owned columns, projected evidence rows, and
a next cursor. Ordering is ascending over the complete evidence key. The
cursor is versioned and bound to the normalized selection and request tenant.
It is not an authorization token and modifying its ordering key cannot widen
the server-owned relation or selection.

`POST /v1/metric-drilldown/export` produces the complete selected population
with the same projected columns as CSV or XLSX. Export is server-side and
rejects results exceeding its row, byte, cell, execution-time, or concurrency
limits. It never silently truncates.

The evidence contract has these limitations:

- Summary-grain silver cannot produce event-grain evidence. AI and
  collaboration currently expose source summaries or derived populations.
  Git exposes commit and pull-request events, task duration metrics expose
  issue events, and wiki page creation exposes page events; their remaining
  measures use the finest summary grain preserved by silver.
- Metric results and evidence are not transactionally snapshot-isolated from
  each other during a dbt rebuild. They reconcile after the complete gold
  build because observations derive from evidence.
- Pagination and each export are bound to the evidence table UUID. A rebuild
  during the operation fails with `EVIDENCE_SNAPSHOT_EXPIRED`; the client must
  restart the selection rather than mix rows from two builds. Previous table
  snapshots are not retained.
- Source links are omitted. Hosted services commonly use custom domains, and
  the current silver contract does not preserve a canonical web base URL. A
  future source registry can add a non-secret `web_base_url` keyed by source
  instance and combine it with provider-specific record identifiers.
- Drilldown preserves the existing metric entity and tenant behavior. This
  change does not add identity-tree authorization or warehouse tenant
  enforcement.

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

Builtin definitions are declared in one declarative registry
(`src/backend/services/analytics/src/domain/metric_definitions/registry.yaml`:
one `sources` list and one `metrics` list) and converged into the DB by a
startup reconciler, not by migrations. Migrations own schema only. The registry
is embedded at build time (`include_str!`) and deserialized once into the seed
types in `builtin.rs`; the reconciler reads it through `builtin_sources()` /
`builtin_metrics()`. Registry invariants (key shape and uniqueness,
input/measure references, computation field combinations, presentation-complete
formats) are pinned by the `builtin.rs` tests, which parse the same embedded
registry — a malformed or drifted registry fails the build.

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
  selection: {
    metric_key: string
    entity: { type: string; ids: string[] }
    period: { from: string; to: string }
    filters: Array<{ dimension: string; values: string[] }>
  }
  drilldown?: {
    granularity: Array<"event" | "source_summary" | "derived_population">
  }
} & (
  | { computation: "sum" }
  | { computation: "ratio"; scale: number }
  | { computation: "median" }
  | { computation: "distinct_count" }
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

Entity ids for `person` are canonical person UUIDs since the identity
cutover: trimmed, parsed as UUIDs (any casing / hyphenless accepted,
canonicalized on echo), deduplicated. A non-UUID value — including the
pre-cutover email shape — is a client error, never a silent empty result, and
so is the nil UUID, which is syntactically valid but never a person.

The id cap counts SUBMITTED ids, before they are parsed: blanks are skipped
and duplicates collapse, so capping the parsed count would let a padded
request through and pay for the parse of every entry first.

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

Entity-level scoping is enforced, not deferred. The caller is resolved from
the gateway JWT, and identity answers in one batch call which of the requested
person ids that caller may see (`POST /v1/visible-persons`: self, active
grants, a tenant-wide wildcard grant, and org-chart descendants). One id
outside the answer refuses the WHOLE request with 403 — never a partial
response, which is indistinguishable from absent data. The check runs before
any ClickHouse work.

Reaching identity is mandatory: an unconfigured or unreachable identity
service is a server error, so an authorization backend that is down can never
read as "permitted". Service principals bypass the gate. `person` is the only
entity type with a rule; any other type fails closed.

Peer views expose aggregates only (no peer entity ids); period, timeseries,
and breakdown views expose per-entity values — for ids the caller is allowed
to see.

Warehouse tenant isolation is compiled into every observation and cohort
read but is OFF BY DEFAULT: the predicate is `tenant_id = ?` only when
`metric_catalog.enforce_tenant_scope` is set, and degrades to a match-all
(`tenant_id = ? OR 1 = 1`) otherwise, because the `tenant_id` stamped at
ingestion has no defined mapping to the control-plane tenant yet — an exact
match would silently empty every metric. Until an environment aligns the
stamp and flips the flag, this runtime is SINGLE-TENANT: with the filter
degraded, a peer pool sharing a `cohort_id` across tenants would mix them,
so multi-tenant deployment is unsupported rather than partially isolated.
Defining the mapping and defaulting the flag on is the multi-tenant unlock.

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

The measure already appears in a managed source. Check the source's `measures`
list in `registry.yaml`, the emitting evidence model, and the observation model
derived from it.

1. Add one entry to the `metrics` list in
   `src/backend/services/analytics/src/domain/metric_definitions/registry.yaml`:
   metric key (`namespace.metric_name`, lowercase snake case), label,
   description, unit, format, direction, entity type, computation, input role
   mapping to the measure, allowed dimensions, peer cohort key.
2. Run `cargo test -p analytics` — the registry invariant tests validate
   key shapes, input/measure references, and computation field combinations.

The reconciler seeds the definition on the next deploy. If every input measure
has healthy evidence metadata, the metric automatically receives drilldown,
table, and CSV/XLSX export support. No drilldown-specific SQL, frontend
configuration, migration, or dbt change is required.

### Case 2: new measure from an existing source

The source exists but does not emit the measure yet.

1. Add the measure to the source's `<family>_metric_evidence` model in
   `src/ingestion/gold/`. Summary measures use the shared shape macros from
   `src/ingestion/dbt/macros/metric_observation_measures.sql`;
   event measures select stable source records into the evidence contract.
   Read only class-contract columns; never vendor-specific ones. If the fact
   is absent from the class contract, extend that contract first (staging
   models declare semantics in their `schema.yml`).
2. Choose the evidence granularity deliberately:
   - emit one stable row per source record for `event`.
   - emit the finest source-retained grouping for `source_summary`.
   - emit the reconstructed participating population for
     `derived_population`.
   Event rows need deterministic `record_id` values and should place reusable
   grouping fields in `dimensions` and human-facing fields in `details`.
3. Derive the matching observation measure from the evidence relation. The
   observation remains the aggregate-ready runtime input; the evidence row is
   the population that explains it.
4. Add the `measure_key` to the observation model's `schema.yml`
   `accepted_values` test.
5. Add the measure key to the source's `measures` list in `registry.yaml` and
   classify its evidence granularity.
6. Add or reuse a source-measure presentation rule when the default
   date-plus-value table is insufficient. Declare detail keys there and add
   explicit column metadata only for fields that are not humanized strings.
7. Add the metric entry as in case 1.
8. Validate: `dbt parse` + `cargo test -p analytics` (see Validation
   commands).

### Case 3: new observation source

The metric family reads data no managed source covers.

1. Create `<family>_metric_evidence` and
   `<family>_metric_observations` dbt gold models in
   `src/ingestion/gold/`, `schema=insight`, `ref()`-ing silver models
   (medallion layering rules:
   `docs/domain/ingestion-data-flow/specs/DESIGN.md`). The evidence model emits
   the evidence contract; the observation model derives the aggregate-ready
   observation contract from it. Document both in
   `src/ingestion/gold/schema.yml`.
2. Add a source entry (source + measures + dimensions) to the `sources` list
   in `registry.yaml`, with `source_ref` and `evidence_ref` set to their
   relation names and every measure assigned an evidence granularity. No
   backend enum or table-name code changes are required: relation names are
   validated data and both contracts are probed by the runtime schema
   validator.
3. Add source-measure presentation rules for event shapes that need
   human-facing detail columns.
4. Add metric entries as in case 1.
5. Validate: `dbt parse` + `cargo test -p analytics` (see Validation
   commands). The runtime schema validator probes the new relation at
   startup.

### Rules that hold for every case

- No metric-key-specific branches in runtime code.
- Evidence presentation branches may depend on source and measure, never on
  final metric key.
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

## Custom Metrics

Runtime-authored metrics are DELIVERED. A person authors a metric with
`origin = 'custom'` through the DB and the `/v1/metrics*` REST surface (not
`registry.yaml`, which stays the builtin seed). Its observation source is
custom SQL (`source_kind = 'custom_observation_sql'`) that joins silver/gold
relations and emits the observation contract below. The runtime executes such
metrics like any other; it no longer stops at a gate that stored-but-inert
definitions once hit.

### Execution wire

The compiler treats a `custom_observation_sql` source's SQL as the observation
relation: the observation `FROM` becomes `(<custom sql>)` — the custom SELECT
wrapped as a subquery — and the generic runtime applies the same bucketing,
aggregation, peer-cohort, and tenant-filter wrapping around it that it applies
to a managed observation relation. Nothing metric-key-specific is added.

### Observation contract the custom SQL must emit

The wrapped SELECT must project the long-format observation columns:

```text
tenant_id, source_key, entity_type, entity_id, metric_date,
measure_key, observed_at, value, subject_key, dimensions
```

Column semantics match the Source Measure Observation Contract above.

### Reconcile-safety of custom rows

The builtin YAML reconciler's `disable_missing` step is scoped to
`origin = 'builtin' AND tenant_id IS NULL`. Custom rows — `origin = 'custom'`,
tenant-scoped — fall outside that predicate, so a reconcile pass converging
builtins to the registry never disables or deletes a custom metric.

### Validation and execution role

Custom SQL passes the existing single-SELECT gate (one `SELECT`/`WITH`, no
DDL/DML) on write and before execution; the gate additionally rejects
external/remote table functions (`remote`, `url`, `file`, `s3`, `mysql`,
`cluster`, the `*Cluster` variants, and the data-lake readers), so a custom
source cannot reach outside the read-only warehouse. It executes as
`presentation_ro`, so a custom metric can read the contract but can never
write, alter, or drop it.

### Tenant safety of custom SQL

The compiler applies the tenant predicate to the rows the wrapped SELECT
**emits**, not to the tables it **reads**. A custom SQL is therefore required to
be **tenant-neutral and row-preserving**: it MUST expose the real `tenant_id` of
every source row (so the outer predicate scopes correctly) and MUST NOT
fabricate, rebind, or constant-fold `tenant_id`, aggregate across tenants before
the outer filter, or otherwise let one tenant's rows influence another's result
or export. The platform enforces this **structurally only in part** — single
SELECT, no external table functions, `presentation_ro`, and the outer tenant
predicate — and does **not** statically prove a given SELECT is tenant-neutral.
That residual trust is the same posture as the saved-query console
(`/v1/queries*`): authorship is trusted, which is why the surface is
experiment-gated and off on production. Cross-tenant aggregate and export/import
behavior is covered by tests.

## Frontend Contract

Frontend collection rendering:

- requests metric keys and views.
- treats the optional `drilldown` capability as the only evidence-action
  switch; there is no frontend metric allowlist.
- forwards the canonical metric selection returned by `/v1/metric-results`,
  narrowing period and dimension filters for chart-point interactions.
- renders server-owned typed evidence columns and rows without interpreting
  the internal evidence contract.
- uses the same canonical selection for table pagination and server-side
  CSV/XLSX export.
- treats configured required views as required.
- normalizes response arrays only for local lookup.
- renders using returned label, description, explanation, unit, format,
  direction, and computation.
- owns chart choice and layout.

Backend responses do not include chart metadata.

## Non-Goals

- No public source labels in metric results.
- No metric-key-specific branches in result compilation.
- No partial responses for oversized results.
