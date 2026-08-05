# Adoption Findings — Semantic Layer

Review of [REFERENCE.md](./REFERENCE.md) + [IMPLEMENTATION.md](./IMPLEMENTATION.md)
against the current codebase and the presentation-layer epic
(constructorfabric/insight#1803). The design is adopted as the Phase B target;
these are the reconciliation notes and the changes it implies to the existing
plan.

## Alignment

The design matches the principles the presentation split already commits to:

- **No invented query language.** The definition format reuses proven shapes
  (MetricFlow envelope, MBQL/JSON-Logic filter trees, allowlisted SQL
  fragments) — a closed composition schema, not a DSL.
- **Raw SQL at exactly one gated layer** (custom datasets), with
  dataset-sized blast radius; the measure/metric/chart layers stay structured.
- **Definitions as data, server-owned semantics, one compiler, capability from
  code+config not from stored rows.**

## Where the shipped registry (#1974) fits

Phase 1 of the implementation plan is "product definitions as repo YAML,
embedded and parsed at compile time, replacing `builtin.rs` constants,
validated in CI." That is exactly what #1974 shipped: the metric registry moved
from Rust literals to `registry.yaml` (`include_str!`, validated by the
registry tests, `deny_unknown_fields`). So **#1974 is the first, shipped step of
this design's Phase 1** — the authoring-as-data move.

Caveat: #1974's YAML still uses the *observation-relation* shape
(`source_ref: *_metric_observations`, per-measure `evidence_granularity`,
reconcile into `metric_source_measures`). The target **rewrites** that store
schema to the dataset/measure/metric domain model and **deletes** the dbt
observation gold models. So #1974's format direction is on-path; its schema
shape is transitional and gets rewritten at cutover. Migration cost stays near
zero because builtin rows are seed-reconciled.

## Changes this implies to the epic sub-issues

- **#1975 (metric passports + drift test)** and the generate-SQL framing of
  **#1976** need re-scoping. The design is compiler-first and explicitly
  rejects "registry-driven emission" and "drift gates for generated SQL"
  because nothing is generated — one compiler over datasets removes the drift
  class those tasks guard. Keep the *passport* idea (provenance surfaced per
  value); drop the *generated-SQL drift gate*.
- **#1976 (semantic raw->derived compiler)** becomes the design's Phase 2
  compiler over datasets, not a raw->derived transpiler.
- **#1977/#1978 (FE rework, query->card promotion)** map onto Phases 4–5
  (discovery API + runtime editing). Promotion of a good ad-hoc query is the
  custom-dataset -> measure -> metric ladder (runtime, role-gated), not a
  bespoke per-card mechanism.
- **#1980 (subtree + row-policy backstop)** is where the caller-scope
  predicates below become compiler-injected row filters.

## Authorization: a scope the design must name

The design specifies the **tenancy** predicate as injected on every compiled
query, and says entity scoping and peer modes are "injected uniformly," but it
does **not name org-chart visibility**. That is a real, required scope and must
be first-class in this document:

- **Security — no people outside your org scope.** A viewer reads per-entity
  values only for individuals within their org-chart scope (self + related
  subtree/reports). People outside it are never returned and their existence is
  not disclosed. The visible set is resolved from the org chart (owned by the
  identity service); a caller-scoped predicate is injected server-side beside
  the tenancy predicate, at the single compiler choke point, and the client can
  never widen it. Fail-closed if the authorization source is unavailable.
- **Scope isolation — no other teams/cohorts.** No per-entity reads for
  unrelated teams/cohorts. Cross-cohort comparison is aggregates-only: peer
  views return distributions with no member ids, suppressed below a minimum
  distinct-member floor so small groups cannot disclose individuals.
- **Cohorts are org-gated, not tag-gated.** A person's peer cohort is drawn
  from within their org-chart scope; tags/attributes refine the cohort *inside*
  that boundary and never pull a person across it. An R&D member and a Sales
  member who share a tag are never placed in the same cohort, so a shared tag
  cannot route around scope isolation — even the aggregate a viewer sees stays
  within org scope, not merely anonymized. The org chart is authoritative over
  cohort composition; tags are secondary and only subdivide within it.

  Current state / gap: only `org_unit` cohorts are *implicitly* org-scoped
  today, because the peer pool is scoped by `cohort_key` value plus tenant
  (`compile_peer_batch_query` / `push_cohort_scope`), never intersected with
  the viewer's org-visible set. An arbitrary or tag-based cohort key would span
  the org boundary. This rule must be enforced where cohort membership is
  produced (the cohort dataset) and re-asserted as the compiler's injected
  scope (#1980), so it holds for every cohort key, not just `org_unit`.

Current state: analytics enforces this at the request boundary
(`domain/person_visibility.rs` -> identity `/v1/visible-persons`), forwarding
the caller's token and refusing out-of-scope ids (leaking only a count). Under
compiler-first this moves into the compiler's shared `WHERE` as an injected
`entity ∈ visible_set` filter (the #1980 work). Definitions stay
scope-agnostic; the scope is injected per request, exactly like tenancy.

These two guarantees are recorded on the epic
(constructorfabric/insight#1803) and are carried into the governed
[DESIGN.md](./DESIGN.md) as named injected scopes (Principles & Constraints plus
the compiler component), alongside the tenancy predicate.

## The real decision

Adopting this design commits to **rewriting the observation-relation store
schema and deleting the dbt observation gold models**, replaced by one compiler
computing over datasets at query-time grain. That deletion — not the YAML
format — is the load-bearing decision. The e2e metric suite is the parity
invariant that makes the cutover safe (same seeds, same requests, same
expectations against the new executor).

## Open items from review (in the reference design)

Points raised against [REFERENCE.md](./REFERENCE.md) that need an author/team
decision; carried here rather than silently rewritten into the reference:

- **Percentile capability is under-specified.** The expressiveness section
  lists percentiles, but the measure aggregation enum
  (`count | sum | avg | min | max | count_distinct`) omits it. Clarify whether
  percentile is a **metric-level computation** (as median is today —
  `ComputationSpec::Median` over event-grain measures) or a measure
  aggregation, and state the read-time contract (percentiles compute over event
  rows, never a percentile-of-percentiles). The governed DESIGN adopts the
  metric-level reading unless the author decides otherwise.
- **Do not serve a superseded custom-dataset table after a semantic version
  bump.** The materialization section both invalidates caches on a version bump
  and allows a failed rebuild to keep serving the previous table. Restrict the
  previous-table fallback to refresh failures **under the same definition
  version**; after a semantic bump, fall back to live compute or return
  `unavailable` — never serve values computed under a superseded definition
  (the section's own "version mismatch means recompute or reject" rule).
- **Gate cache reads on definition availability.** The cache read decision
  checks policy, version, and coverage but not availability; an `unavailable`
  definition must not serve cached rows. Require `availability == available`
  before cached or live execution, else return the stored unavailable error.

## Governance status

Adopted as governed `cfs` `sdlc` artifacts under
`docs/domain/semantic-layer/specs/`: [PRD.md](./PRD.md) and
[DESIGN.md](./DESIGN.md) are the template-conformant, registered artifacts;
[REFERENCE.md](./REFERENCE.md) is the detailed design narrative they distill and
cite for depth, and [IMPLEMENTATION.md](./IMPLEMENTATION.md) is the migration
plan. The reference and implementation docs are kept verbatim as adopted so the
governed specs never lose the original rationale.
