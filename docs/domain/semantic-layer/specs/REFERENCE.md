# Technical Design — Semantic Layer

Status: **adopted target architecture** for Phase B of the presentation-layer
split (epic constructorfabric/insight#1803) — the detailed design rationale.

This is the in-depth reference narrative. The governed, template-conformant
artifacts are [PRD.md](./PRD.md) (product requirements) and
[DESIGN.md](./DESIGN.md) (technical design); they distill this narrative and
reference it for depth. The migration sequence is in
[IMPLEMENTATION.md](./IMPLEMENTATION.md); the adoption review and open items are
in [FINDINGS.md](./FINDINGS.md). Read this before changing metric definitions,
the compiler, or the definition store.

The semantic layer is the system through which every analytical value is
defined, validated, computed, and served. Definitions — measures, metrics,
charts, dashboards — are data, editable at runtime by authorized users
through structured editors. Semantics are owned and executed by the server.
Storage, caching, and materialization are private implementation details
behind the definition contract.

## Principles

1. **Definitions are data.** Anything a user may eventually edit is a row in
   a definition store, never code. Product-shipped definitions and
   user-created definitions are the same kind of object with different
   ownership.
2. **Truth is base facts plus definitions.** Any derived value is a cached
   evaluation of a definition over base facts. Caches are invalidated by
   definition version; they are never the contract.
3. **One executor.** A single compiler turns definitions into queries.
   Two interpreters of one definition format diverge — null handling, time
   zones, deduplication — so a second executor is only ever a bounded
   transitional state.
4. **Capability comes from code and configuration, never from data.** What
   can be asked is a function of shipped connectors and stored definitions.
   A tenant with zero ingested rows has full authoring capability. Data
   determines only which concrete values exist.
5. **No invented languages.** Every layer of the definition format reuses a
   proven external shape. The system owns one small composition schema, not
   an expression language.
6. **Semantics are server-owned.** Clients request by key and render what
   they are given. Meaning — filters, formulas, windows, formatting
   identity — never lives in a client.
7. **The safety boundary is structural.** Users compose from typed
   catalogs through closed grammars. Free-form SQL exists at exactly one
   gated layer with dataset-sized blast radius, or not at all.

## Domain Model

```text
dataset                    product-owned relation; the correctness boundary
  └─ field catalog         typed, role-annotated schema; the authoring palette
       └─ measure          aggregation of one dataset; lowest editable layer
            └─ metric      composition of measures; transform, format, direction
                 └─ chart / dashboard   presentation only
```

### Dataset

A dataset is a queryable relation with guaranteed semantics: deduplicated,
tenant-scoped, with stable column names and types. Datasets carry the entire
correctness burden of ingestion — mutability handling, late data, source
quirks — so that nothing above them ever re-solves those problems.

Datasets are product-owned, with one gated exception — custom datasets —
specified below.

### Custom datasets

A holder of a distinct dataset-author role (per-tenant enablement, off by
default — authoring SQL is a sharper privilege than composing measures)
may register a SQL SELECT as a dataset. This is the sanctioned home for
everything the measure layer deliberately cannot express: cross-dataset
joins, sequence and window logic, nested aggregation, correlated
comparisons.

The contract:

- **Read surface.** Custom SQL reads the catalog'd datasets — which are
  the row-level relations themselves (class contracts and purpose-built
  relations), every row, every contract column — through dedup-safe
  views rather than the underlying physical tables. Same rows, governed
  door: the raw table objects, raw source payloads, and pipeline
  intermediates are not referenceable. This makes read discipline
  unbreakable by construction (an author cannot accidentally query
  around deduplication) and preserves ingestion's freedom to rework
  everything beneath the contract. Within that surface, full SQL —
  joins, window functions, subqueries, nested grouping.
- **Registration.** The statement is parsed and its references checked
  against the catalog; the result schema is captured, and the author
  annotates result columns with catalog roles (entity, dimension,
  measurable, event time). The annotated schema becomes an ordinary
  catalog entry; measures compose on top structurally.
- **Execution.** Always wrapped: the tenancy predicate is applied from
  outside the statement, and resource guardrails (timeout class, memory,
  row caps) bound it. The product guarantees safety and isolation;
  semantic correctness of the SQL is the author's, and lineage marks
  everything built on a custom dataset so consumers see the provenance.
- **Lifecycle.** Versioned with revisions like every definition;
  availability states apply (an upstream product dataset changing shape
  sends the custom dataset and its dependents to `unavailable` through
  the same reconcile loop); deletion is blocked while measures reference
  it. Query-time by default, promotable to the version-keyed cache when
  slow.
- **Chaining.** A registered custom dataset is a catalog entry, so
  custom datasets may build on other custom datasets. Definitions form a
  DAG: cycles are rejected at registration, and availability cascades —
  an upstream dataset going `unavailable` takes its dependents with it,
  each state carrying the root cause. Deep chains are the primary
  candidates for cache promotion, since each layer is otherwise a
  stacked view resolved at query time.
- **Blast radius.** A defective custom dataset breaks itself and its
  dependents, visibly — never the measure engine, never another
  dataset.
- **Forking.** Any dataset offers "derive custom dataset", prefilling
  the editor with a starting point from its displayed body (product
  bodies, shown as compiled SQL, may need mechanical adaptation to the
  catalog read surface — the fork is a head start, not a guaranteed
  verbatim copy). Combined with shadow definitions this is the sanctioned
  runtime hotfix path when a product definition is believed wrong: fork
  the dataset, fix it, build tenant measures and metrics on top, repoint
  charts — corrected numbers with zero deploys, while product truth
  stays untouched under its own keys. It is a stopgap by design: forks
  do not track upstream changes, the divergence is surfaced on the fork
  ("derived from X @ build N, upstream changed since"), and the expected
  end state is a product fix followed by reverting the repoint and
  deleting the fork.

Datasets are the retention boundary. A measure can be evaluated only as far
back as its dataset's history reaches, and that horizon is part of the
served contract ("data available from …"), not an implementation surprise.
Retention is therefore a priced product commitment: measurable columns are
not pre-aggregated away on the assumption that no future definition will
need them.

### Field catalog

Every dataset publishes a typed catalog of its fields, each annotated with a
role:

- **entity** — identifies who or what a row is about (person, team,
  repository). Entity fields bind rows to the entity model used for
  scoping and cohorting.
- **dimension** — groupable; a stable value key paired with a display
  label.
- **measurable** — numeric, aggregatable.
- **event time** — timestamp candidates for time bucketing.

The catalog is generated from dataset schemas, never hand-maintained. It is
simultaneously the editor's palette, the compiler's validation universe, and
the discovery API's vocabulary. A field absent from the catalog does not
exist, at any layer above.

Dimensions with the same key across datasets are **conformed**: `repository`
means the same identity space wherever it appears. Conformance is declared
in the catalog, and it is what makes cross-measure composition and shared
filters meaningful.

### Measure

A measure is a declarative aggregation of one dataset — the lowest editable
layer, and the atom every metric is built from.

```yaml
key: large_prs_merged
dataset: git_pull_requests
description: Merged pull requests changing at least 500 lines.
filter:
  all:
    - { field: state,         op: eq,  value: merged }
    - { field: lines_changed, op: gte, value: 500 }
aggregation: count          # count | sum | avg | min | max | count_distinct
value_expr: null            # SQL fragment over catalog fields where the
                            # aggregation takes an operand
event_time: merged_at
entity: author
dimensions: [repository, source]
```

The format is a composition of proven shapes; none of it is invented:

- **Envelope** — the dbt MetricFlow semantic-model shape: aggregation as a
  closed enum, expression slots, an explicit aggregation time dimension.
  Every corner case has a reference answer in a published spec rather
  than a design meeting.
- **Filter grammar** — structured predicate trees in the MBQL / JSON Logic
  style: `all` / `any` / `not` combinators over `{field, op, value}`
  leaves, operators from a closed enum. Fields must resolve in the
  dataset's catalog with a compatible type.
- **Scalar expressions** — SQL fragments, not a bespoke expression
  language. Each fragment is parsed (warehouse dialect) and admitted only
  if its AST contains exclusively: catalog column references, literals,
  arithmetic, and allowlisted functions. Subqueries, table references, and
  settings are rejected at the parser. SQL's fully specified semantics —
  nulls, types, precedence — come for free; safety comes from the
  allowlist, and every widening of the allowlist is an explicit reviewed
  change.

A measure's `dimensions` list is its dimension capability. There is no
separate capability registry: capability is a projection of definitions,
so it cannot drift from them.

### Metric

A metric composes measures into a served value: a computation over one or
more inputs (direct, ratio, derived expression over named inputs), an
optional post-aggregation shaping stage (affine transform, clamping), and
display identity (direction, format, naming). The MetricFlow metric types
are the reference vocabulary.

A metric's dimension capability is derived, not declared: the intersection
of its inputs' dimension sets, where dimension-agnostic inputs (constants,
global denominators) are identity elements that do not shrink the set.

Cross-dataset metrics compose **at the aggregated level**: each input
measure aggregates within its own dataset, and the compiler joins the
aggregates on entity, time bucket, and conformed dimensions. Row-level
joins across datasets are not a measure-layer capability; where a genuine
need exists, the join is built as a product-owned dataset and measured on
top.

### Chart and dashboard

Pure presentation: which metric, which view (timeseries, breakdown, single
value, table), which dimension, layout, thresholds, targets. No query
semantics of any kind. These are the first definitions to become
user-editable, because they are configuration over already-validated
capability.

## Expressiveness and Its Limits

The editable layer intentionally covers the aggregate-compositional class:
filtered counts, sums, averages, extrema, distinct counts, percentiles,
arbitrary arithmetic composition of those, over any catalog field, by any
declared dimension, at any grain, with windows, cohort comparison, and
shaping. This matches the expressiveness class the surviving semantic
layers converged on.

What the editable layer deliberately cannot express, and where each case
goes instead — the rule being that anything row-relational or
order-dependent is dataset work below the line, while anything
aggregate-compositional stays above it. The first four are solvable
without product involvement wherever custom datasets are enabled: a
dataset author builds the dataset, and measures compose on top.

- **Row-level cross-dataset relationships** → a joined dataset
  (product-built, or custom), measured on top.
- **Sequence-dependent logic** (funnels, durations between events,
  streaks) → a dataset that computes the ordering into columns
  (intervals, stage timestamps); measures aggregate the result.
- **Second-order aggregation** (aggregate, regroup, aggregate again) →
  a dataset holding the inner aggregation, or a compiler query mode if
  the shape recurs broadly.
- **Correlated logic** (a row compared against its group's own
  statistic) → a dataset with the baseline precomputed as a column.
- **Facts not ingested** → connector work; no layer conjures data.
- **As-of-history semantics** → snapshot-modeled datasets; measures see
  only what the dataset preserves.
- **Non-affine shaping** → extension of the transform vocabulary, a
  reviewed product change.

Nothing useful is unreachable; the ladder decides who builds it and with
what safety guarantees. Every escape lands in the dataset layer or below,
where full SQL and ingestion-grade review are available — never in a
loosened editor.

### Capability by declaration

Structure is not all-or-nothing. All logic — joins, filters, computed
values, windows — may live in dataset SQL; what a definition *declares*
determines which platform features it receives, and every declaration is
optional. An absent declaration switches features off; it never rejects
the definition:

| Declared | Unlocks |
| --- | --- |
| result schema only | rendering as a table or tile — legal, feature-dead |
| event-time field | timeseries, grains, windows, time zones |
| entity field | entity scoping, cohorts, peer comparison |
| dimensions (value/label bindings) | breakdowns, filters, discovery pickers |
| aggregation, platform-applied | re-aggregation across grain and scope, the rollup cache, composition into metrics |

Two things can never move into the SQL, because they are what the
platform operates on: the final aggregation (embedding it bakes grain and
scope into a string and kills re-aggregation, caching, and composition)
and column roles (not reliably inferable from a statement). Everything
else is authoring preference. A fully declared measure is therefore not a
restriction on SQL — it is the SQL's type signature, five fields with no
logic in them, and it is the minimum structure that keeps the feature set
mechanical.

Declarations are validated data, deliberately not row-shape conventions:
a declaration violation fails at write time with a precise error; a shape
convention fails at query time, in production.

This grammar is also the intended surface for machine authorship. A
closed, catalog-validated definition language is a strictly better
target for AI assistants and connected agents than freeform SQL:
violations are machine-checkable and repairable, semantics are diffable
and auditable, and the editing API doubles as the agent tool surface.

### Runtime completeness

The runtime surface is complete over the class contracts: with custom
datasets enabled (including chaining), the entire product metric catalog
is expressible at runtime with no product code. Shipping product
definitions as reviewed code is therefore a governance choice — review,
data tests, CI, e2e coverage for semantics that deserve them — never a
capability boundary. The same metric is buildable either way; only its
ownership and assurance level differ.

## Time Model

Time is compiler-owned and uniform across every measure:

- Each measure aggregates over its declared `event_time`.
- Grain (day, week, month, quarter) is a query-time parameter, not a
  property of stored data. No grain is baked into materializations that
  the contract depends on.
- Bucketing happens in a single declared reporting time zone per tenant;
  the same request never changes meaning across executors or cache tiers.
- Windowed and cumulative semantics (trailing windows, to-date) are
  compiler features parameterized at request time, not encoded into
  definitions ad hoc.

## Capability Model

"What can be queried" has three independent layers. Conflating them produces
either dead editors on empty tenants or API contracts that drift with data.

1. **Product capability** — what shipped datasets and definitions can
   express. A function of code and product seeds. Identical for every
   tenant, including one with zero ingested rows.
2. **Tenant configuration** — which definitions this tenant has created or
   enabled. A function of the definition store. Runtime-editable.
3. **Data values** — which concrete repositories, categories, people
   exist. A function of ingested rows. Serves filter value pickers only.

Dimension *keys* (what can be grouped by) come from layers 1–2. Dimension
*values* (what can be filtered to) come from layer 3, through a dedicated
distinct-values endpoint with an explicit unavailable state for unscanned or
oversized value sets. Capability is never inferred from stored derived rows:
an empty installation would report no capability, partial ingestion would
mutate the contract, and an ingestion defect would silently remove features.

## Compiler

The compiler is the single executor: definition plus request (entity scope,
date range, grain, dimensions, filters) in, warehouse SQL out. It owns:

- catalog resolution and type checking,
- filter-tree and expression-AST rendering,
- per-dataset read discipline (deduplication, mutable-read handling),
  inherited from dataset metadata rather than re-decided per measure,
- dimension-capability validation,
- tenancy predicates on every query it emits,
- the time model,
- the post-aggregation transform stage.

Product and user definitions compile through the same path; a product
measure is not a special case, only a differently owned row.

## Materialization

Materialized results are a compiler-managed cache tier, invisible to the
contract: the cache stores **work, not answers**. It holds each measure's
aggregated rows at the finest served grain, and any request shape is
served from them by re-aggregation (coarser grains, breakdowns,
composition, peer modes). One cached representation covers the entire
request space — the warehouse pre-aggregation pattern, as opposed to a
request→response cache, which pays off only when identical questions
repeat. A short-TTL response cache may sit on top for repeated identical
requests; it is an orthogonal add-on, never the mechanism for coverage or
correctness.

What a cached row is depends on the aggregation kind, because the math
dictates the finest reusable form:

- additive aggregations (`count`, `sum`, `avg` components, `min`, `max`) —
  one row per entity × time bucket × dimension tuple;
- percentile-family — one row per source event (a median of medians is
  wrong; percentiles compute at read over event rows);
- distinct-count — one row per counted subject, so cross-period distinct
  counts stay exact.

Physical layout is one shared cache relation for all measures — measure
results share one shape, so tenant-created measures never trigger DDL —
partitioned by `(measure key, definition version, time bucket)` so that
invalidation and refresh are atomic partition operations. Per-measure
relations are rejected: they bind schema operations to user actions and
grow unbounded with tenants × measures.

**Caching is policy, not semantics.** Whether a measure is materialized
lives in a cache policy (enabled, refresh schedule, hot window, coverage
watermarks) separate from the definition; toggling it never changes the
definition version, because nothing about meaning changed. Product
definitions may ship a default policy; tenant definitions start uncached
and are promoted by policy change. Only measures are cached — metric
composition is cheap re-aggregation at read time, and caching terminal
results would multiply invalidation surface for no coverage gain.

Refresh is scheduled, never triggered by reads: the read path stays
read-only, and no request ever pays a build or stampedes one. Because
datasets are mutable (late events, history rewrites), each refresh rebuilds
a recent hot window atomically by partition replacement; settled periods
stay put; a dataset reprocessing forces a full rebuild through the same
path. Coverage watermarks advance only after data lands, so the read path
never trusts a hole.

**Dataset materialization.** Custom datasets follow the same principles
with row-level physics. Unmaterialized by default (a stored SELECT
resolved at query time), promoted by the same kind of cache policy when
slow. Because a dataset's schema is author-defined, each materialized
dataset gets its own relation per definition version — the one place DDL
is bound to a runtime object, bounded by happening only on promotion
(a rate-limited policy event, never on save) and only for promoted
datasets. Refresh is a full rebuild with an atomic swap: incremental
refresh cannot be inferred safely from arbitrary SQL over mutable
upstreams, so always-correct-and-expensive is the default and windowed
rebuild is a later author-declared opt-in. Version bumps invalidate
exactly as for measures (reads fall back to the live view). Refresh
walks the definition DAG topologically — upstream datasets before
dependent datasets before dependent measure caches — triggered by
ingestion completion like the availability probe. A failed rebuild keeps
serving the previous table and alarms: data staleness is a surfaced,
tolerable cache property ("as of" timestamps); semantic staleness never
is. Rebuilds run under the same resource guardrails as queries, so a
pathological definition fails its promotion loudly rather than becoming
a scheduled incident.

The read decision, made per input measure independently: policy enabled,
cached version equals current definition version, and the requested range
inside the coverage watermarks → read cache; otherwise compile live from
the dataset. A single metric may mix cached and live inputs in one query.
Every degraded state — stale version, uncovered range, disabled policy —
falls back to live compute: slower, never wrong.

Every cached row is stamped with the definition version it was computed
under. Version mismatch means recompute or reject — never silently serving
values computed under a superseded definition. History depth equals dataset
retention at evaluation time, in every tier.

## Definition Store

Definitions live in application-owned storage with these invariants:

- **Versioning.** Semantic changes increment `definition_version`;
  presentation-only changes do not. Versions drive cache invalidation and
  audit. The version is never hand-maintained: every write path (seed
  reconciliation, editing API) canonicalizes the definition's semantic
  fields, compares against the stored body, and bumps on difference with a
  compare-and-set — a forgotten manual bump would mean new semantics
  served from an old cache, so the possibility is removed structurally.
  Versions are strictly monotonic and never reused, even when a change
  restores an earlier body: reusing a version would let a partially
  invalidated cache present holes as truth. Correctness never depends on
  cache cleanup: reads are version-keyed, so superseded cache entries are
  logically dead the moment the version bumps, and physical cleanup is
  unhurried background work.
- **Referential integrity.** Metrics reference measures; charts reference
  metrics; dashboards reference charts. Deletion is blocked while
  referenced or explicitly cascades. Keys are stable; display names change
  freely.
- **Ownership and tenancy.** Product definitions are tenant-invariant and
  read-only for tenants. Tenant definitions are tenant-scoped and
  namespaced so they can never shadow product keys. Editing rights are a
  role, per layer of the domain model.
- **Auditability.** Definition history is retained: who changed what,
  when, from which version. Runtime editability without audit is not
  acceptable in a multi-tenant product.
- **Fail closed on the control plane only.** A missing or unreadable
  definition store is a startup error — the one dependency the service
  cannot start without. "No definitions found" must never present as "no
  capability". Warehouse state is never a startup gate: deploys ship
  code and definitions only, and the warehouse converges on its own
  schedule.
- **Warehouse divergence is a state, not a crash.** Product definitions
  are validated in CI against the declared dataset contracts from the
  same commit, so a definition referencing an undeclared field cannot
  ship. At runtime the structural probe reconciles declared contracts
  with the live warehouse: an absent relation (bootstrap, ingestion not
  yet run) or a relation lagging the shipped contract puts the affected
  definitions into an explicit `unavailable` state — excluded from
  serving with a precise error, visible in discovery, self-healing when
  the warehouse catches up; divergence persisting past a threshold is an
  operations alarm. The probe is a continuous background reconcile loop
  over warehouse metadata (with an ingestion-completion trigger as a
  latency optimization, never the guarantee), distinct from
  seed reconciliation, which runs at startup because its input changes
  only on deploy. Definition availability state is stored with the
  violation; serving and discovery read the stored state and never probe
  inline. Tenant definitions invalidated by contract changes
  get the same state, surfaced to the tenant's administrators.
- **One schema, defined once.** The definition format is defined as typed
  code in the owning service; authored YAML and stored rows are
  serializations of those types, and parser, validators, compiler, and
  editing APIs consume the same types. Machine-readable schema for
  external tooling is exported from them, never hand-maintained.
- **Validation before existence.** Product definitions are parsed and
  semantically validated at build time (an invalid definition fails the
  build), structurally validated against the warehouse at startup; tenant
  definitions get the identical validators at API write time. An invalid
  definition is never written.

Product defaults ship as seeds and bootstrap into the store; from then on
the store is the single runtime source of truth for every consumer,
including whatever build-time machinery materializes caches.

## Discovery API

The server tells clients what exists; clients never hardcode or probe:

- the catalog of metrics and measures visible to the tenant
  (product ∪ tenant), with computation, format, direction, and allowed
  dimensions per entry,
- for editors: dataset field catalogs with roles and types, including
  each dataset's definition body for display — custom datasets from their
  store row, product datasets as a build-generated artifact of the
  shipped model SQL, read-only and stamped with the build it came from.
  One source of truth per body is preserved: the display copy is derived
  at build time, never authored, and never executed,
- on demand: distinct dimension values from data, paginated, with an
  explicit unavailable state.

Requests are validated against the same catalog the discovery API serves,
so the two can never disagree. The validation error path exists for stale
or handcrafted clients; the designed path is pickers that render only valid
choices.

## API Surface

Four groups; the query contract is stable, the rest additive.

- **Query** — request by metric key with entity scope, range, grain,
  dimensions, filters, view. Responses are self-describing and carry
  provenance: definition version, cache-or-computed, data-available-from,
  availability state.
- **Discovery** — the catalog of visible metrics and measures with their
  capability; dataset field catalogs with roles, retention, origin,
  display body, and lineage; paginated dimension values with an explicit
  unavailable state. Everything a client renders comes from here.
- **Definitions** — role-gated CRUD per layer, running the same
  validators as seed reconciliation with precise field-level errors;
  validate-only dry runs; fork; cache-policy changes. Semantic writes
  bump versions and append revisions; deletes are blocked with the
  referencing keys while referenced. This surface is also the agent/MCP
  tool surface — machine authors get the same endpoints and the same
  validation, never a separate path.
- **Dashboards** — a dashboard definition resolved with its charts and
  their metrics' current capability in one read.

The frontend is a renderer of definitions: dashboards, pickers, editors,
and error states all derive from discovery and dashboard payloads. It
holds no metric vocabulary, no per-chart dimension lists, no validation
semantics, and synthesizes no metric copy — server-owned meaning becomes
structural rather than conventional. Adding or changing a metric, chart,
or dashboard is a data change that ships no frontend code.

## Editability Boundary

| Layer | Editable by | Mechanism |
| --- | --- | --- |
| datasets | product | ingestion code |
| custom datasets | dataset-author role, default off | registered SQL SELECT over catalog'd datasets |
| field catalog | nobody | derived from dataset schemas |
| measures | admin | structured editor over the field catalog |
| metrics | admin | structured editor over measures |
| charts / dashboards | admin, optionally end users | structured editor over metrics |

The boundary is the product's answer to "how much BI freedom": full
composition freedom above the dataset line, none below it. Moving the line
is a design decision, never an incremental widening.

## Adoption Order

Independent of any current implementation, the layers come online in
dependency order, each shipping value alone:

1. **Definitions as data.** All product measures and metrics exist as
   store rows in the target format, whatever still executes them.
   Versioning, integrity, and namespacing land here, when they are cheap.
2. **Single compiler on the read path.** The compiler serves product
   definitions, shadow-verified for parity against whatever it replaces;
   alternative executors retire on cutover.
3. **Runtime editing, presentation first.** Dashboards and charts, then
   metrics, then measures — each layer opening only after the one below it
   is served by the compiler and covered by discovery.

## Alternatives Considered

- **Deriving capability from stored derived rows.** Rejected: empty
  tenants report no capability, partial ingestion mutates the contract,
  defects silently remove features, discovery scans data.
- **Parallel declarations (emission-side and serving-side).** Rejected:
  two sources of truth with no cross-check fail silently — unreachable
  dimensions or valid-but-empty groups.
- **A complete invented expression DSL.** Rejected: invented languages
  fail in their underspecified corners (nulls, typing, time); every
  needed layer has a proven donor shape, so the system owns only a small
  composition schema.
- **Embedding an external semantic-layer engine.** Rejected: an
  additional service with its own caching, auth, and tenancy model to
  operate and reconcile. Schema ideas are adopted; the executor is owned,
  because it must enforce warehouse-specific read discipline and tenancy.
- **A key-value response cache (Redis-style) as the materialization
  tier.** Rejected: it caches exact request→response pairs in a
  combinatorial request space (low hit rate), cannot be joined or
  re-aggregated by the warehouse, holds bulk rows at memory prices, and
  invalidates by TTL and key patterns — the fuzzy invalidation that
  serves stale semantics. Response caching remains available as an
  orthogonal short-TTL layer on top.
- **Client-composed queries as the flexibility mechanism.** Rejected:
  metric meaning fragments across clients, there is no governance seam
  for access control, transforms, caching, or performance guardrails, and
  storage schema becomes public API.
- **Users author SQL at the measure layer** ("why structure at all, if
  the SQL hatch exists?"). Rejected because guardrails make SQL safe to
  run, not understood — and every platform feature requires
  understanding the definition: request-time grain and timezone need a
  known event-time field and re-bucketable aggregation; dimension
  capability, discovery, and validation derive from declared dimensions;
  entity scoping and peer modes are injected uniformly; the rollup cache
  depends on knowing the aggregation kind (a SQL result is opaque —
  response-cacheable at best); contract changes are mechanically traced
  to structured definitions but are string archaeology in SQL; and the
  measure editor's audience is not SQL-literate. A metric authored as
  SQL opts out of all of it — the documented tax of Metabase native
  questions, and why Looker confines SQL to derived tables. The dividing
  rule: SQL produces rows (datasets), structure produces meanings
  (measures, metrics) — the layers the platform must operate on stay
  structured; the layer that only yields a schema may be SQL.

## Risks

- **Allowlist calibration.** Too narrow blocks legitimate measures; too
  wide leaks unsafe constructs. Start narrow; widen only by reviewed
  additions, which the parser makes explicit.
- **Label instability.** Display labels ride data; a renamed entity
  yields conflicting labels for one value key. The compiler owns one
  resolution rule (latest observed label wins) stated in the read
  contract.
- **Deploy and definition skew.** Additive capability changes are safe
  mid-rollout; removals transiently reject valid requests and require a
  deprecation window.
- **Editor-induced load.** Runtime-created measures compute at query time
  until promoted; a pathological definition is a performance event, not a
  correctness event. The compiler enforces guardrails (row limits,
  timeout classes) as part of the contract.
- **Scope creep toward a general BI platform.** The editability boundary
  table is the line. Changes to it are explicit design decisions with
  this document updated first.
