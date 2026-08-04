# Implementation Plan — Semantic Layer

Status: proposal. Companion to `DESIGN.md` (target architecture). This
document decides what is kept, rewritten, or deleted, and sequences the
work. Unlike the design, it references current code on purpose: it is the
migration document.

The strategy is **compiler-first**. The runtime compiler over datasets is
the target's center of gravity, so it is built early and everything the
target does not need is deleted early — instead of investing in machinery
that keeps the interim architecture consistent while it waits to be
demolished. Concretely rejected on that basis: generating dbt observation
models from measure definitions, drift gates for generated SQL, and
registry-driven emission. All of that would harden the layer the target
retires.

## Current State (inventory)

- **Definition store** — MySQL via sea-orm
  (`src/backend/services/analytics/src/migration/m20260625_000001_metric_definitions.rs`
  and follow-ups): `metric_sources` (observation relation registry with
  schema-probe status machinery), `metric_source_measures` (measure keys
  only — semantics live in dbt SQL), `metric_source_dimensions`,
  `metric_definitions` (computation `sum|ratio|median|distinct_count`,
  format, direction, tenant scoping, `origin builtin|custom`),
  `metric_definition_inputs`, `metric_definition_dimensions`. Builtin
  definitions are Rust constants
  (`domain/metric_definitions/builtin.rs`, incl. affine + clamp shaping)
  reconciled at startup (`seeds.rs`).
- **Emission** — dbt gold models
  (`src/ingestion/gold/*_metric_observations.sql`) hand-author measure
  semantics through shape macros
  (`src/ingestion/dbt/macros/metric_observation_measures.sql`); day grain
  baked into the observation contract (`metric_date Date`).
- **Validation** — `domain/metric_definitions/validator.rs` +
  `domain/schema_validator/`: relation/column probes plus dimension
  coverage **probed from observed rows** (empty tenant → `Unchecked`).
- **Serving** — `domain/metric_results/` compiles
  period/timeseries/peer/breakdown/histogram queries over observation
  relations; `/v1/metric-results`, `/v1/metrics/queries`.
- **Tests** — declarative e2e suite
  (`src/ingestion/tests/e2e/metrics/*.test.yaml`) seeds bronze and asserts
  through the batch endpoint; dbt tests on observation shapes.

## Metric Lifecycle, Before and After

The shortest way to see what changes. Adding one metric ("large PRs
merged"):

Today:

1. Hand-write a CTE + shape-macro call in the family's gold observation
   model, dimension array included — the meaning, as dbt SQL.
2. Declare the measure key, metric, allowed dimensions, format, direction
   in `builtin.rs` — the meaning again, as Rust.
3. Deploy both; dbt pre-computes observation rows; startup seeds the
   store; the validator probes observed rows to confirm dimensions.
4. Serving reads the pre-computed rows at their baked day grain.
5. The chart is frontend code; changing it is a third deploy.

Target:

1. Write one YAML definition (dataset, filter, aggregation, event time,
   dimensions) — the only place meaning exists. Later, an admin does the
   same in an editor, writing a store row instead of a repo file.
2. The write path validates it against the field catalog and expression
   allowlist; invalid definitions cannot exist.
3. Nothing is pre-computed. Capability is read off the definition;
   an empty tenant exposes it identically.
4. The compiler turns definition + request into SQL over the dataset at
   any grain; materialization appears later, per measure, as a
   version-keyed cache — purely a latency decision.
5. The chart is a definition row referencing the metric; no deploy.

Three deploys and two hand-synchronized restatements become one reviewed
definition; pre-computation moves from mandatory-and-authored to
optional-generated-versioned.

## Keep / Rewrite / Delete

| Component | Verdict | Reason |
| --- | --- | --- |
| bronze → silver ingestion, class contracts | **keep** | the dataset layer; carries dedup/mutability correctness the target builds on |
| purpose-built gold relations (state intervals, cohorts) | **keep** | become named datasets; derived-column logic belongs below the measure layer |
| declarative e2e suite + batch endpoint | **keep** | the cutover invariant: same seeds, same requests, same expectations against the new executor |
| public API contract (`/v1/metric-results`, `/v1/metrics/queries`) | **keep** | response shape is presentation contract, orthogonal to how values are computed |
| metric composition semantics (ratio/median/distinct-count, affine + clamp) | **keep semantics, rehome code** | proven definitions; they become data and compile in the new executor |
| store technology + conventions (MySQL, sea-orm, forward-only migrations, seed reconciliation) | **keep** | conventions are fine; the schema is not |
| store schema (`metric_sources` … `metric_definition_dimensions`) | **rewrite** | shaped around observation relations, not the domain model; measures are key-only; probe-status machinery solves a drift problem the target removes structurally |
| `metric_results` query builder | **rewrite** | its input model is observation rows at day grain; the compiler's input model is datasets at query-time grain — a second input path bolted on would preserve the wrong core |
| observation gold models + shape macros | **delete after cutover** | measure semantics move to definitions; materialization returns later as compiler-owned cache, not as authored models |
| row-probing dimension validation (`check_dimension_coverage`, `Unchecked` state, `dimension_not_covered` plumbing) | **delete** | capability must not depend on data; replaced by structural validation at startup and CI |
| `metric_sources.source_ref` relation-name-as-data + column probe machinery | **delete** | replaced by the dataset registry and catalog artifact |

Migration burden of the store rewrite is near zero by construction: builtin
rows are startup-reconciled from code, so new tables repopulate from seeds.
Any `origin = 'custom'` rows get a one-shot mapping migration.

## What Compiler-First Unlocks Immediately

- Query-time grain (week/month/quarter without day-grain assumptions) and
  tenant-timezone bucketing from day one — impossible over the baked
  observation contract.
- New measure correct instantly with zero materialization — the editing
  story's core mechanic, proven years before an editor exists.
- One executor from the start: no dual-execution window, no
  Jinja-vs-compiler semantic drift to manage, no parity infrastructure
  living forever.
- No generated-SQL machinery: nothing to codegen, gate, or keep from
  drifting, because nothing is generated.

## Phase 1 — Definition Core

The new store, authored definitions, and validators. No serving change.

1. **Schema, new tables** (one migration, domain-shaped):
   - `datasets` — key, database + relation, read discipline (dedup
     strategy), retention horizon, `origin` (`product` | `custom`).
   - `measures` — key, dataset ref, filter (JSON, closed-enum MBQL-style
     tree), aggregation ENUM(`count`,`sum`,`avg`,`min`,`max`,`count_distinct`),
     `value_expr` / `subject_expr` (validated SQL fragments), `event_time`
     field, entity field, dimension bindings (dimension key →
     value/label catalog fields), `definition_version`, tenant scoping +
     `origin` as today.
   - `metrics` — key, computation over role-typed measure inputs,
     transform (affine + clamp), format, direction, entity type, cohort
     key; same scoping.
   - `definition_revisions` — append-only audit (kind, id, version, actor,
     body snapshot) written by every mutation path from the first day the
     store exists, so no later retrofit.
   - CHECK-constraint style follows the existing migrations (key shapes,
     aggregation/expr biconditionals); the probe-status columns are not
     carried over.
2. **Field catalog, minimal form.** The catalog generator (Rust, per
   Decision 3) ships here, not in Phase 4: dataset field names, types,
   and roles from dbt `schema.yml` `meta:` blocks, emitted as a
   build-time artifact. Phase 1 needs it as the validation universe;
   Phase 4 only extends it into the discovery surface.
3. **Validators.** `domain/definitions/expr.rs`: `sqlparser` (ClickHouse
   dialect) + AST allowlist (catalog columns, literals, arithmetic,
   allowlisted functions; everything else rejected). Filter tree as serde
   types with closed operator enum. One write path — seed reconciliation
   and future editing APIs share these validators.
4. **Authoring.** Product definitions as repo YAML
   (`src/ingestion/measures/<source_key>.yaml` and
   `metrics/<group>.yaml`), embedded and parsed at compile time, replacing
   `builtin.rs` constants; a build-time test runs the full validator set
   over every file (shape, enums, expressions, cross-references), so an
   invalid product definition fails CI and cannot ship; startup seed
   reconciliation as today. Repo YAML
   is how product definitions are authored and reviewed; the store is the
   single runtime source.
5. **Extraction.** Transcribe every measure from the gold observation
   models into YAML against silver/gold datasets. Logic that exceeds the
   measure schema (attribution tiers, file categorization) is not forced
   into filter trees — it moves to (or stays in) named dataset relations.
   The `presence` macro family is decomposed during extraction
   (count-distinct over the event-time day) rather than becoming an
   aggregation kind.
6. **Structural validation.** Two stages replacing the row-probing path
   conceptually (the old path keeps serving until cutover). CI: definitions
   validate against the field catalog derived from dbt `schema.yml` at the
   same commit — contract mismatches cannot ship. Runtime: the probe
   reconciles contracts with the live warehouse; absent or lagging
   relations put affected definitions into an explicit self-healing
   `unavailable` state, alarmed if persistent, never a startup failure and
   never a silent capability reduction. The probe is a continuous
   background loop over warehouse metadata (interval + backoff — the shape
   of today's `schema_validator` loop, retargeted from row probing to
   contract reconciliation), with an orchestrator poke after dbt runs for
   fast healing; availability state is stored on the definition row and
   read by serving and discovery, never probed inline. This also removes today's
   deploy-time coupling: no dbt build runs at deploy, deploys ship code
   only, and the warehouse converges asynchronously.

Exit: every product measure and metric exists as validated YAML
reconciled into the new store; old system untouched and serving.

## Phase 2 — Compiler and Cutover

1. **Compiler.** `domain/compiler/`: definition + request (entity scope,
   date range, grain, dimensions, filters) → ClickHouse SQL over
   datasets. Owns filter/expression rendering from validated ASTs,
   per-dataset read discipline (FINAL/dedup from dataset metadata),
   tenancy predicates, grain bucketing in the tenant reporting time zone,
   metric composition (all four computation types), the transform stage,
   and guardrails (row caps, timeout classes, memory settings) so a
   pathological definition degrades to an error, not an incident.
2. **Parity, then cutover, per source family.** "Family" (git, tasks,
   wiki, collab, ai) is a migration-only unit — it is how the old
   observation tables, seeds, and e2e fixtures are already grouped, so it
   is the natural granule for flipping executors coherently. The concept
   retires with the old system in Phase 3; steady-state decisions
   (materialization, refresh) are per measure, never per family. The e2e
   suite is the invariant: same bronze seeds, same requests, expectations green against
   the compiler path. A temporary flag selects executor per family;
   shadow-compare on live traffic per family until divergence classes
   (dedup, timezone, null propagation) are resolved deliberately; flip;
   next family. Shadow runs double as the latency measurement that
   triggers Decision 1's cache pull-forward. The flag and comparison harness are explicitly temporary
   — weeks per family, deleted at Phase 3, not a permanent dual-execution
   capability.
3. **Materialization where measured, not assumed.** Cutover requires
   parity and acceptable latency. Where query-time compute over datasets
   misses the latency budget for a family, the compiler gains its cache
   tier early: materialized results keyed by
   `(definition key, definition_version)`, refreshed by
   backend-scheduled jobs, reads falling back to compilation on version
   mismatch or cache miss. This is the target's cache design pulled
   forward on demand — never the old observation contract kept alive.

Exit: every product metric serves through the compiler; observation
relations receive no reads.

## Phase 3 — Deletion

Immediately after the last family cuts over, while context is fresh:

- gold observation models and shape macros, their dbt shape tests, and
  the observation build step;
- the old serving query builder in `metric_results/` (request validation
  and response DTOs remain with the API);
- row-probing validator machinery, `Unchecked` state,
  `dimension_not_covered` plumbing, backoff/probe scaffolding that
  existed to reconcile definitions with observation relations;
- old store tables, after the one-shot migration of any custom rows;
- the per-family executor flag and parity harness.

Exit: one executor, one store schema, no dead branches. The system is
smaller than before the project started.

## Phase 4 — Catalog and Discovery

1. **Field catalog, full form.** The Phase 1 artifact (names, types,
   roles) extends with everything discovery needs: value↔label pairing
   surfaced per dimension, conformed dimension keys, display metadata.
   Same generator, same `schema.yml` `meta:` source, same build-time
   transport — until custom datasets exist, catalogs change only with
   product deploys.
2. **Discovery API.** Extend `domain/catalog/`: metrics and measures with
   derived allowed dimensions (intersection over inputs,
   dimension-agnostic inputs as identity elements), dataset field
   catalogs for editor scope, distinct-dimension-values endpoint
   (paginated, explicit `unavailable` state, served from the cache tier
   or bounded dataset scan).
3. **Frontend adoption.** Pickers consume discovery; no client-side
   capability knowledge, per the server-owned-meaning boundary.

Exit: an editor could be built against discovery alone; empty tenants
report full product capability.

## Phase 5 — Runtime Editing

Product order: dashboards/charts → metrics → measures.

1. **Chart/dashboard definitions.** `chart_definitions`,
   `dashboard_definitions` (metric ref, view type, dimension, layout,
   thresholds), tenant-scoped, product seeds from today's hardcoded
   dashboards, `ON DELETE RESTRICT` to metrics, revisions written like
   every other definition.
2. **Editing APIs.** CRUD per layer, role-gated, tenant-namespaced keys,
   sharing Phase 1 validators (one write path). Deletion blocked while
   referenced.
3. **Editors.** Chart/dashboard first (configuration over validated
   capability), then metric editor over measures, then measure editor
   over dataset field catalogs. Each layer opens only when the layer
   below is discovery-covered.
4. **Custom datasets (gated, last, optional).** Admin-authored SELECT
   registered as a dataset: parsed, validated, schema captured into the
   catalog; default off. Catalog transport moves from build-time artifact
   to store-backed at this point, not before.

Exit: an administrator creates a measure, composes a metric, places it on
a dashboard, and sees correct data — zero deploys, full audit history.

## Cross-Cutting

- **The e2e suite is the safety rail for the rewrite.** It pins behavior
  through Phases 2–3 (executor swap, deletion) without pinning
  implementation; new fixtures cover compiler-only behavior (grain
  variants, timezone edges, adversarial expressions rejected by
  validators).
- **API stability.** `/v1/metric-results` and `/v1/metrics/queries`
  contracts do not change during the rewrite; discovery grows additively
  in Phase 4.
- **Migrations** forward-only per convention; the store rewrite is
  additive tables + seed repopulation + late drops, never in-place
  mutation of the old schema.
- **Observability.** Compiler logs definition key + version + tier
  (computed/cache) per query; shadow divergence and validator rejections
  are first-class metrics during Phase 2.

## Dependencies

```text
Phase 1 ──► Phase 2 ──► Phase 3 (deletion)
                │
                └──► Phase 4 ──► Phase 5
```

Phase 4 needs only Phase 1 definitions plus compiler-derived capability,
so it can start during late Phase 2. Phase 5 chart/dashboard work can
start once Phase 4 discovery serves metrics.

## Decisions

1. **Materialization is exception, not default.** Query-time compute is
   the default for every family. A family gets the cache tier pulled
   forward only if its shadow-phase p95 on the batch endpoint exceeds the
   current path's p95 by more than a fixed margin agreed before shadow
   starts. The shadow harness already runs both executors on live
   traffic, so the latency evidence arrives for free; no separate
   measurement project.
2. **Reporting time zone is a tenant profile setting**, default UTC, with
   no per-request override. A per-request time zone makes the same
   request mean different things per client, which violates server-owned
   semantics. Exploration-style TZ shifting, if ever wanted, is a new
   explicit API capability — not a bucketing parameter.
3. **The catalog generator is Rust**, sharing the backend's definition
   and catalog parsers. A generator in another language is a second
   parser of the same source of truth — a small instance of exactly the
   duplication this project exists to remove. The extra build wiring is
   trivial against that.
4. **Peer comparison is a compiler query mode; cohort membership is a
   dbt dataset.** Peer comparison evaluates the same metric over cohort
   members and aggregates the results — request-shaped, not
   definition-shaped — so it belongs to the compiler like grain does,
   parameterized at request time. Cohort membership is derived data with
   ingestion-grade correctness concerns, which is what datasets are for.
