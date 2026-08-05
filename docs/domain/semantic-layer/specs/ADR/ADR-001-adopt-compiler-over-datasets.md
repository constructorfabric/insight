---
status: accepted
date: 2026-08-05
---

# ADR-001: Adopt one compiler over datasets (definitions-as-data) for Phase B

**ID**: `cpt-semantic-layer-adr-adopt-compiler-over-datasets`

<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [One compiler over datasets, definitions-as-data (the semantic layer)](#one-compiler-over-datasets-definitions-as-data-the-semantic-layer)
  - [Incremental dual-authoring — transpiler plus generated-SQL drift gates](#incremental-dual-authoring--transpiler-plus-generated-sql-drift-gates)
  - [Externalize semantics to a warehouse-native or third-party semantic layer](#externalize-semantics-to-a-warehouse-native-or-third-party-semantic-layer)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

## Context and Problem Statement

Phase A of the presentation-layer split (epic constructorfabric/insight#1803) drew a read-only, tenant-scoped contract and shipped the first authoring-as-data step: the metric-definition seed moved from Rust constants to a validated YAML registry (#1974). Metric *meaning*, however, is still expressed twice — once as dbt SQL that pre-computes observation gold, and once as a Rust query builder over those observations — and capability is probed from stored rows, so a tenant with no ingested data reports no capability.

Phase B must decide how metric semantics are defined and executed: keep evolving the two-place authoring model incrementally, or collapse meaning into a single definition-plus-compiler path. The choice determines whether the remaining Phase B sub-issues (#1975–#1978) build on generated-SQL machinery or on a compiler, and it is largely irreversible once cutover deletes the old path.

## Decision Drivers

* Meaning must live in exactly one place — dbt SQL plus Rust constants can diverge on null handling, time zones, and deduplication.
* Capability must be a projection of shipped code and stored definitions, not of observed rows, so an empty tenant retains full authoring capability.
* A single executor must serve every definition, so there is no second interpreter to drift against.
* Every emitted query must carry the same server-injected scopes (tenancy, org-scope entity visibility, cohort isolation) at one choke point the client cannot widen.
* Authorized users must add or change a metric, chart, or dashboard without a code deploy.

## Considered Options

* One compiler over datasets, definitions-as-data (the semantic layer)
* Incremental dual-authoring — keep dbt observation gold plus the Rust builder, add a raw-to-derived transpiler and generated-SQL drift gates
* Externalize semantics to a warehouse-native or third-party semantic layer (dbt Metrics, Cube, or similar)

## Decision Outcome

Chosen option: **"One compiler over datasets, definitions-as-data (the semantic layer)"**, because it is the only option that collapses meaning to a single place, makes capability a projection of definitions rather than of stored rows, and removes the generated-SQL drift class entirely instead of guarding it. Definitions (datasets, measures, metrics, charts, dashboards) become data; one compiler turns a definition plus a request into warehouse SQL; the injected scopes live at that one choke point. The migration is staged (definition core, then compiler and per-family cutover, then deletion of the observation path, then discovery and runtime editing), with an end-to-end parity suite as the invariant at each executor flip.

### Consequences

* Good, because metric meaning exists once as a reviewed definition; the dbt/Rust divergence class disappears rather than being policed.
* Good, because capability derives from definitions, so a tenant with zero ingested rows still has full authoring capability.
* Good, because tenancy, org-scope entity visibility, and cohort isolation are injected uniformly at the single compiler choke point and no definition or client input can widen them.
* Good, because authoring a measure, metric, or dashboard becomes a runtime, role-gated, audited change with no deploy.
* Bad, because it is a multi-phase migration with a temporary dual-execution window (an executor-selection flag plus shadow comparison per source family) that must be built and then deleted.
* Bad, because the cutover ends in an irreversible deletion step — the observation gold models, their seeds, and parts of the `metric_results` builder are removed once each family reaches parity.

### Confirmation

Cutover is gated per source family by an end-to-end parity suite: the same bronze seeds, the same requests, and the existing expectations must stay green against the compiler path before the executor flips (`cpt-semantic-layer-nfr-executor-consistency`). Read discipline is confirmed by adversarial tests that assert no read bypasses dataset dedup and that the validator rejects raw-table references (`cpt-semantic-layer-nfr-source-read-discipline`). Scope injection is confirmed by adversarial tests proving no definition or client input widens the tenancy, org-scope, or cohort predicates. The staged sequence and its exit criteria are tracked in [IMPLEMENTATION.md](../IMPLEMENTATION.md).

## Pros and Cons of the Options

### One compiler over datasets, definitions-as-data (the semantic layer)

Definitions are data; a single compiler renders definition + request to SQL; storage, caching, and materialization are private behind the definition contract. Full rationale in [REFERENCE.md](../REFERENCE.md).

* Good, because there is one source of truth for meaning and one executor.
* Good, because capability and authorization are structural, derived from code and definitions at compile time.
* Good, because expressiveness the structured layers cannot reach lands in a single gated custom-dataset SQL layer with dataset-sized blast radius, not a loosened editor.
* Neutral, because materialization is added per measure only where shadow-phase latency evidence warrants it, not assumed up front.
* Bad, because it requires the largest migration and a disciplined, temporary dual-execution cutover.

### Incremental dual-authoring — transpiler plus generated-SQL drift gates

Keep dbt-emitted observation gold and the Rust builder; layer a raw-to-derived transpiler over the #1974 registry and add drift gates to keep the two authorings in sync.

* Good, because it reuses the shipped observation gold and query builder with the least immediate disruption.
* Bad, because meaning stays authored twice; drift gates police a divergence the target design eliminates outright.
* Bad, because capability stays tied to stored rows, so empty tenants keep reporting no capability.
* Bad, because scopes remain enforced at request boundaries rather than uniformly at one compiler choke point.

### Externalize semantics to a warehouse-native or third-party semantic layer

Push metric semantics into dbt Metrics, Cube, or a comparable external semantic layer.

* Good, because it avoids building an in-house compiler.
* Bad, because server-owned scope injection (tenant, org-scope, cohort) and capability-from-definitions are not first-class in these tools, so the security-critical invariants would be re-implemented around them anyway.
* Bad, because it introduces an external runtime and definition format the owning service does not control, contradicting the code-owned-schema constraint.

## More Information

This ADR records the decision behind the already-adopted target architecture documented in [DESIGN.md](../DESIGN.md), [PRD.md](../PRD.md), [REFERENCE.md](../REFERENCE.md), and [IMPLEMENTATION.md](../IMPLEMENTATION.md); the adoption review is in [FINDINGS.md](../FINDINGS.md). It re-scopes the epic's Phase B sub-issues: #1974 is the first shipped step of the definition core (in a transitional observation-relation shape the target rewrites at cutover); #1976 becomes the design's Phase 2 compiler over datasets rather than a raw-to-derived transpiler; the metric-passport idea (#1975) survives as per-value provenance, while the generated-SQL drift gate it introduced belongs to the transitional model and retires when the compiler removes the drift class. #1977/#1978 map onto the discovery API and runtime-editing phases.

## Traceability

- **PRD**: [PRD.md](../PRD.md)
- **DESIGN**: [DESIGN.md](../DESIGN.md)

This decision directly addresses the following requirements or design elements:

* `cpt-semantic-layer-fr-one-compiler` — the decision to route every definition through a single compiler is exactly this requirement's mechanism.
* `cpt-semantic-layer-fr-definitions-as-data` — choosing definitions-as-data over code-literal semantics realizes this requirement.
* `cpt-semantic-layer-constraint-sql-only-at-dataset-layer` — the decision confines free-form SQL to the gated dataset layer, honoring this constraint.
* `cpt-semantic-layer-nfr-executor-consistency` — the per-family shadow-compare-then-flip cutover is how this decision keeps executor parity.
* `cpt-semantic-layer-nfr-source-read-discipline` — compiler-owned reads from dataset metadata are how this decision preserves dedup discipline.
* `cpt-semantic-layer-design-semantic-layer` — this ADR is the recorded rationale for the overall design element.
