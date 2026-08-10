# Technical Design — Identity Resolution


<!-- toc -->

- [1. Architecture Overview](#1-architecture-overview)
  - [1.1 Architectural Vision](#11-architectural-vision)
  - [1.2 Architecture Drivers](#12-architecture-drivers)
  - [1.3 Architecture Layers](#13-architecture-layers)
- [2. Principles & Constraints](#2-principles--constraints)
  - [2.1 Design Principles](#21-design-principles)
  - [2.2 Constraints](#22-constraints)
- [3. Technical Architecture](#3-technical-architecture)
  - [3.1 Domain Model](#31-domain-model)
  - [3.2 Component Model](#32-component-model)
  - [3.3 API Contracts](#33-api-contracts)
  - [3.4 Internal Dependencies](#34-internal-dependencies)
  - [3.5 External Dependencies](#35-external-dependencies)
  - [3.6 Interactions & Sequences](#36-interactions--sequences)
  - [3.7 Database Schemas & Tables](#37-database-schemas--tables)
- [4. Additional Context](#4-additional-context)
  - [4.1 Min-Propagation Algorithm (ClickHouse-Native)](#41-min-propagation-algorithm-clickhouse-native)
  - [4.2 Matching Engine Phases](#42-matching-engine-phases)
  - [4.3 Operator Corrections (Merge / Split / Bind / Exclude)](#43-operator-corrections-merge--split--bind--exclude)
  - [4.4 Analytics Integration (mirror + resolve macro)](#44-analytics-integration-mirror--resolve-macro)
  - [4.5 End-to-End Walkthrough: Anna Ivanova](#45-end-to-end-walkthrough-anna-ivanova)
  - [4.6 End-to-End Walkthrough: Andrei Sokolov (Min-Propagation)](#46-end-to-end-walkthrough-andrei-sokolov-min-propagation)
  - [4.7 Deployment](#47-deployment)
  - [4.8 Operational Considerations](#48-operational-considerations)
- [5. Implementation Recommendations](#5-implementation-recommendations)
  - [REC-IR-01: ClickHouse atomicity for merge/split — SUPERSEDED](#rec-ir-01-clickhouse-atomicity-for-mergesplit--superseded)
  - [REC-IR-02: Incremental watermark for identity inputs (open)](#rec-ir-02-incremental-watermark-for-identity-inputs-open)
  - [REC-IR-03: Shared unmapped table for all domains — RESOLVED](#rec-ir-03-shared-unmapped-table-for-all-domains--resolved)
  - [REC-IR-04: Temporary tenant and source ID derivation via sipHash128 (Phase 1)](#rec-ir-04-temporary-tenant-and-source-id-derivation-via-siphash128-phase-1)
  - [REC-IR-05: Explicit canonical id emission per connector (Phase 2)](#rec-ir-05-explicit-canonical-id-emission-per-connector-phase-2)
- [6. Traceability](#6-traceability)

<!-- /toc -->

- [ ] `p3` - **ID**: `cpt-insightspec-ir-design-identity-resolution`

> Version 3.0 — August 2026
> Sync with the implemented journal architecture (append-only `persons` observation log per ADR-0002; operator corrections per ADR-0003; reviewed design in constructorfabric/insight#2180). The v2.0 ClickHouse-native alias/matching architecture is retained as explicitly-marked future material.
---

## 1. Architecture Overview

### 1.1 Architectural Vision

Identity Resolution maps disparate identity signals — emails, usernames, employee IDs, platform-specific handles — from all connected source systems to canonical persons. It answers one question for every downstream consumer: "which person does this source account belong to?"

The implemented architecture is a **journal model** with one source of truth and derived read paths:

- Connectors emit identity observations into ClickHouse `identity_inputs` (evidence; unchanged from v2.0).
- The **persons-seed** (a subcommand of the Rust `identity-resolution` service, run as a scheduled job) folds new observations into the append-only MariaDB **`persons`** observation log: reuse an account's existing binding; else link the group to the person its e-mail already maps to (`LinkedByEmail`); else mint a new person; accounts without e-mail are skipped. It never merges two existing persons. Known gap: when accounts of one e-mail group are bound to *different* persons, the current seed collapses the group onto the first binding and can thereby silently re-derive a binding — the hardening that makes it respect per-account (in particular operator-authored) bindings ships with the manual-resolution feature.
- **Operator corrections** (merge, detach/split, bind, exclude — ADR-0003, reviewed design in constructorfabric/insight#2180) are appended to the same `persons` journal as binding observations authored by a real operator UUID — the same currency the seed reads, so durability requires no parallel store; the seed hardening above closes the one path that could overwrite them.
- The service's **persons-sync** worker republishes the journal into ClickHouse (`identity.identity_persons`) via an atomic table swap; the dbt `resolve_person_id` macro resolves `person_id` at build time from that mirror. The v1 macro resolves **by e-mail only** (latest `value_type='email'` observation per normalized e-mail); the manual-resolution feature upgrades it to **account-first with e-mail fallback** (§4.4) — required for corrections, which are `value_type='id'` bindings, to reach gold.
- `account_person_map` (MariaDB) is a derived SCD2 cache rebuilt from the journal — never a source of truth.

The v2.0 ClickHouse-native plan — a resolution store in `aliases` plus `match_rules`/`unmapped`/`conflicts`/`merge_audits` tables operated by a BootstrapJob/MatchingEngine/ResolutionService pipeline — **was not built**. Its matching-engine material is retained as future direction (§4.1, §4.2, §3.7 future tables); its snapshot-based merge/split mechanism is superseded by the journal-based operator flow of ADR-0003 (§4.3).

This domain is deliberately narrow: it owns the account-to-person binding (the `persons` journal, its seed and sync, and the correction API) plus the `identity_inputs` evidence contract. Golden record assembly, person-level conflict detection, availability, and org hierarchy belong to the person and org-chart domains, which consume `persons` observations.

### 1.2 Architecture Drivers


#### Functional Drivers

| Requirement | Design Response |
|---|---|
| Collect identity observations from all connectors | `identity_inputs` table — each connector writes one row per changed identity value (implemented) |
| Bind source accounts to persons | persons-seed fold into the append-only `persons` journal: reuse binding / link by e-mail / mint / skip (implemented; divergent-group hardening ships with manual resolution) |
| Resolve `person_id` for analytics | persons-sync mirror + dbt `resolve_person_id` macro at build time (implemented — v1 resolves by e-mail only; account-first upgrade with e-mail fallback ships with manual resolution, §4.4) |
| Person lookups for backend consumers | `identity-resolution` service read API over `persons` (implemented; see component spec) |
| Correct wrong groupings (merge / split / bind / exclude) | Operator correction verbs appending to the `persons` journal — ADR-0003, design in #2180 (planned, this iteration; includes the resolver upgrade and seed hardening) |
| Surface what needs operator attention | Review queue derived from `identity_inputs` evidence joined with current `persons` bindings (pending, contested, no-evidence accounts) + resolution-rate shares; no status tables (planned, this iteration) |
| Never link ambiguously | Planned with manual resolution: an e-mail claimed by more than one person stops being linking evidence — no auto-link, surfaced for review (today the divergent-group collapse can pick a winner silently) |
| Configure matching rules per tenant | `match_rules` + MatchingEngine (future — not built; §4.2) |
| GDPR hard erasure of identity data | Purge flow with `alias_gdpr_deleted` archive (future — not built) |

#### NFR Allocation

| NFR ID | NFR Summary | Allocated To | Design Response | Verification Approach |
|---|---|---|---|---|
| `cpt-ir-nfr-alias-lookup-latency` | Person lookup < 50 ms p99 | `persons` indexes + identity-resolution service | Hot-path index `idx_value_id (insight_tenant_id, value_type, value_id)`; bulk analytical resolution happens in dbt at build time, off the request path | Benchmark service lookups under sustained load |
| `cpt-ir-nfr-bootstrap-throughput` | Seed processes 100K inputs/run | persons-seed | Batched ClickHouse reads + batched `INSERT IGNORE` writes | Load test with 100K `identity_inputs` rows |
| `cpt-ir-nfr-bootstrap-idempotency` | Seed re-runs produce no duplicates | persons-seed | `uq_person_observation` natural-key UNIQUE (keyed by `created_at`, migration 004) + `INSERT IGNORE`; known-account rule keeps bindings stable | Run seed 3x on same data; verify row counts |
| `cpt-ir-nfr-no-fuzzy-autolink` | Zero false-positive auto-merges | Structural (no auto-merge path exists); future MatchingEngine | The seed has no branch that merges existing persons; fuzzy rules (future) never auto-link | Seed re-run tests; audit test when matcher ships |
| `cpt-ir-nfr-tenant-isolation` | No cross-tenant data leaks | All tables + service auth | `insight_tenant_id` scoping on keys and journal queries. **Known gap**: the seed's evidence read applies no tenant predicate (hashed producer-side tenant ids — see the note in the reader); single-tenant deployments only until the filter is restored | Cross-tenant resolution query returns empty |
| `cpt-ir-nfr-gdpr-erasure` | Hard purge within SLA | Future purge flow | Not implemented; see §3.7 future tables | Deferred with the purge flow |
| `cpt-ir-nfr-merge-reversibility` | Corrections are auditable and reversible | `persons` journal (ADR-0003) | Append-only history with author + reason on every row; undo = counter-action; full pre-correction state always reconstructible | Correction + counter-action round-trip test |

**Key decision records** (drivers of this architecture):

- `cpt-ir-adr-stable-person-id` — [ADR-0002](ADR/0002-stable-person-id-via-persons-observations.md): stable `person_id` via the append-only `persons` observation journal; three-mode seed binding; known-account rule.
- `cpt-ir-adr-operator-decisions-as-observations` — [ADR-0003](ADR/0003-operator-decisions-as-persons-observations.md): operator corrections are ordinary journal observations; no separate decision store in v1; snapshot-based merge/split superseded.
- `cpt-ir-adr-shared-unmapped` — [person-domain ADR-0001](../../person/specs/ADR/0001-shared-unmapped-table.md): one shared operator queue across identity and person domains (realised in v1 as the derived review queue; a shared persistent store may return with the matcher).

**Implemented PRD requirements anchored by this design**: `cpt-ir-fr-accept-bootstrap-inputs` (the `identity_inputs` evidence intake, §3.7) and `cpt-ir-interface-analytics-resolution` (the mirror + macro analytics read path, §4.4). Incremental seed processing (`cpt-ir-fr-bootstrap-incremental`) is open: each run folds the full evidence set (REC-IR-02).

### 1.3 Architecture Layers

- [ ] `p3` - **ID**: `cpt-insightspec-ir-tech-layers`

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│                         IDENTITY RESOLUTION DOMAIN                           │
├──────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  CONNECTORS               EVIDENCE                    JOURNAL (truth)        │
│  ──────────               ────────                    ───────────────        │
│                                                                              │
│  ┌──────────┐      ┌────────────────────┐      ┌────────────────────────┐   │
│  │ GitLab   │─────▶│                    │      │  persons (MariaDB)     │   │
│  │ GitHub   │      │  identity_inputs   │      │  append-only           │   │
│  │ Jira     │      │  (ClickHouse)      │      │  observation log       │   │
│  │ BambooHR │─────▶│                    │      │  author + reason       │   │
│  │ ...      │      └─────────┬──────────┘      └───▲──────────┬─────────┘   │
│  └──────────┘                │ reads                │ appends  │             │
│                    ┌─────────▼──────────┐          │          │             │
│                    │  persons-seed      │──────────┘          │             │
│                    │  (3-mode fold,     │   bindings          │             │
│                    │  never rebinds)    │                     │             │
│                    └────────────────────┘                     │             │
│                                                               │             │
│  ┌───────────────────┐  appends decisions                     │             │
│  │ Operator API      │──────────────────────────────────────▶ │             │
│  │ bind/merge/detach │  (ADR-0003, planned)                   │             │
│  │ /exclude + queue  │                                        │             │
│  └───────────────────┘                                        │             │
│                                                               ▼             │
│  DERIVED READ PATHS:   account_person_map (SCD2 cache, MariaDB)              │
│                        identity.identity_persons (CH mirror, persons-sync)  │
│                        dbt resolve_person_id macro → gold metrics           │
│                        identity-resolution service read API                 │
└──────────────────────────────────────────────────────────────────────────────┘
```

| Layer | Responsibility | Technology |
|---|---|---|
| Ingestion (evidence) | Connectors write identity observations to `identity_inputs` | ClickHouse (MergeTree) |
| Binding (automation) | persons-seed folds new observations into `persons` (reuse / link-by-e-mail / mint / skip); scheduled runs, guarded and journaled in `operations` | Rust (`seed` subcommand of the identity-resolution service) |
| Corrections (human) | Operator verbs append binding observations to `persons` (ADR-0003; design #2180) | Rust (identity-resolution service, planned endpoints) |
| Storage (journal) | `persons` (append-only observation log — source of truth), `account_person_map` (derived SCD2 cache), `operations` (admin-operation journal) | MariaDB (InnoDB) |
| Mirror + analytics | persons-sync republishes the journal to `identity.identity_persons`; dbt `resolve_person_id` macro resolves at build time | ClickHouse + dbt |
| API | Person lookup (`/v1/profiles`, `/v1/visible-persons`) + planned operator resolution endpoints | Rust (`identity-resolution` service, axum) |
| Future | MatchingEngine over `match_rules` with confidence-scored proposals | Not built (§4.2) |

---

## 2. Principles & Constraints

### 2.1 Design Principles

#### Alias-Centric Resolution

- [ ] `p2` - **ID**: `cpt-insightspec-ir-principle-alias-centric`

Identity resolution is fundamentally an alias mapping problem. Every identity signal from every source system is an alias — an `(value_type, value)` pair that maps to a person. The architecture treats all signals uniformly: an email, a username, an employee ID, and a platform-specific handle are all aliases with different types. This uniform treatment simplifies the resolution pipeline and makes adding new alias types a configuration change, not an architecture change.


#### Storage Split — ClickHouse Evidence + MariaDB Journal

- [ ] `p2` - **ID**: `cpt-insightspec-ir-principle-ch-native-v2`

Evidence is analytical, decisions are transactional. Connector observations (`identity_inputs`) and the read-side mirror of the journal (`identity.identity_persons`) reside in ClickHouse — event-stream-scale, append-heavy, consumed by dbt at build time. The `persons` observation journal, its derived `account_person_map` SCD2 cache, and the `operations` admin journal live in MariaDB and are owned by the Rust `identity-resolution` service — the binding decisions need transactional writes, row-level operator access, and audit-friendly history, and the dataset is tenant-metadata-scale. See §3.7 and ADR-0002 / ADR-0003; migrations are service-owned per ADR-0006.

(v1 of this principle placed the whole resolution store — `aliases`, `match_rules`, `unmapped`, `conflicts`, `merge_audits` — in ClickHouse; that architecture was not built and its tables remain future material, see §3.7.)


#### Append-Only Journal — Corrections Are New Facts

- [ ] `p1` - **ID**: `cpt-insightspec-ir-principle-append-only-journal`

The `persons` journal is never updated or deleted; every change — automated binding or operator correction — is a new appended observation carrying its author and reason. Current state is derived (latest binding per account wins); history is always intact. Operator decisions are written in the same currency the automation reads, so no parallel override store is needed; their durability rests on one seed invariant — *an account's existing binding is reused, never re-derived* — which holds today for consistently-bound groups and is extended to divergent groups by the manual-resolution seed hardening. Undo is a counter-action, never a destructive revert.

One deliberate exception lives outside the decision journal: GDPR right-to-erasure. Append-only governs **identity decisions**; lawful erasure of stored identity values is an explicit administrative operation (future purge flow — see the purge use case in the PRD) whose execution is recorded in the `operations` journal. Erasure is not a decision about who someone is and never flows through binding semantics.


#### Domain Isolation

- [ ] `p2` - **ID**: `cpt-insightspec-ir-principle-domain-isolation`

Identity resolution owns alias-to-person mapping and the `persons` / `account_person_map` identity-history tables, and nothing else. Person-level golden-record assembly, person-level conflict detection, org hierarchy, and assignments belong to their respective domains. This boundary is enforced by table ownership: identity resolution writes only to its own tables and references `persons.person_id` as the logical FK target for aliases.


#### Fail-Safe Defaults

- [ ] `p2` - **ID**: `cpt-insightspec-ir-principle-fail-safe`

Contested identities must never be decided silently. Today the seed auto-links a new account only when its e-mail unambiguously maps to one existing person (`LinkedByEmail`); the manual-resolution feature extends this principle to the contested cases: an e-mail claimed by more than one person stops being linking evidence (no auto-link, surfaced for operator review), and divergent-group collapse is replaced by surfacing. Activity of unresolved accounts appears in analytics with an honest NULL `person_id` — absent rather than misattributed — and never blocks the pipeline. The review queue is **derived from two sources joined on the account key** — the `identity_inputs` evidence (every observed account, including e-mail-less ones) and the current bindings in `persons` — never a status table: an item disappears the moment a decision removes its condition. Accounts with no usable identity evidence are skipped by binding automation but stay visible as no-evidence queue items — unresolved is a first-class, countable state, never hidden. (ADR-0002's `reason='pending-iresolution'` quarantine marker is reserved vocabulary for this flow; the current seed does not yet emit it.)


#### Conservative Matching

- [ ] `p2` - **ID**: `cpt-insightspec-ir-principle-conservative-matching`

Deterministic matching first (exact email, exact HR ID). Fuzzy matching is opt-in per rule and **never triggers auto-link** — always routes to human review. Rationale: false-positive merges are the costliest failure mode of identity systems — fuzzy name matching is a known source of them, so it may suggest but never link.


### 2.2 Constraints

#### Storage Split: Evidence in ClickHouse, Journal in MariaDB

- [ ] `p2` - **ID**: `cpt-insightspec-ir-constraint-storage-split-v2`

ClickHouse holds the evidence and read-side tables: `identity_inputs` (connector observations) and `identity.identity_persons` (the persons-sync mirror of the journal consumed by dbt). The legacy `identity.aliases` table also physically exists in ClickHouse but is not part of the resolution path (see §3.7).

The journal tables — `persons` (append-only observation log), `account_person_map` (derived SCD2 cache), `operations` (admin-operation journal) — are stored in MariaDB (see §3.7): a transactional store is the right fit for binding decisions, row-level operator corrections, and audit history. Schema is owned and applied by the `identity-resolution` Rust service itself via its embedded SeaORM `Migrator` (see [ADR-0006](../../ingestion/specs/ADR/0006-service-owned-migrations.md)).


#### PR #55 Naming Conventions

- [ ] `p2` - **ID**: `cpt-insightspec-ir-constraint-naming`

All tables and columns follow the PR #55 glossary naming conventions:
- Table names: plural (`aliases`, `conflicts`, `match_rules`)
- PK: `id UUID DEFAULT generateUUIDv7()`
- Tenant: `insight_tenant_id UUID`
- Source: `insight_source_id UUID` + `insight_source_type LowCardinality(String)`
- Source account: `source_account_id String`
- Temporal: `effective_from` / `effective_to` (not valid_from/valid_to, owned_from/owned_until)
- Observation: `first_observed_at` / `last_observed_at` (not first_seen/last_seen)
- Actor: `actor_person_id UUID` (not performed_by VARCHAR)
- Timestamps: `DateTime64(3, 'UTC')`
- Soft-delete: `is_deleted UInt8`
- Booleans: `is_` prefix, `UInt8`
- Strings: `String` or `LowCardinality(String)` for low-cardinality
- No `Nullable` unless semantically needed; use empty string or zero sentinel
- Signal naming: `value_type` / `value` (not signal_type/signal_value)


#### Domain Boundary Constraints

- [ ] `p2` - **ID**: `cpt-insightspec-ir-constraint-domain-boundary`

Identity resolution owns:
- The `identity_inputs` evidence contract and table in ClickHouse (§3.7), and the `identity.identity_persons` mirror published by persons-sync
- The MariaDB `persons` journal, `account_person_map` cache, and `operations` journal (§3.7), the persons-seed that populates them (ADR-0002), and the operator correction flow that appends to them (ADR-0003)
- The legacy ClickHouse `aliases` table and the future matching/conflict/merge tables, if and when built (§3.7 future tables)

Identity resolution does NOT own or write to:
- The Person-domain **golden record** (derived from `persons` observations by the person domain)
- `person_availability` (person domain)
- `org_units` table (org-chart domain)
- `person_assignments` table (org-chart domain)
- Any permission/RBAC tables (separate domain)

The person domain reads `persons` observations to build its golden record; identity resolution links aliases to `person_id`s. `person_id` is a random UUIDv7 minted at first observation of each source-account and persisted in `account_person_map`; it is the stable join key across both domains and never re-derived from mutable attributes (see ADR-0002).


#### No Fuzzy Auto-Link

- [ ] `p2` - **ID**: `cpt-insightspec-ir-constraint-no-fuzzy-autolink`

Fuzzy matching rules (Jaro-Winkler, Soundex) MUST NEVER trigger automatic alias creation. They may only generate suggestions for human review. This constraint is non-negotiable: false-positive merges corrupt attribution and are costly to unwind.


#### Half-Open Temporal Intervals

- [ ] `p2` - **ID**: `cpt-insightspec-ir-constraint-half-open-intervals`

All temporal ranges use `[effective_from, effective_to)` half-open intervals. `effective_from` is inclusive (`>=`), `effective_to` is exclusive (`<`). `BETWEEN` is prohibited on temporal columns. `effective_to = '1970-01-01'` (zero sentinel) means "current / open-ended" in ClickHouse (no Nullable).


---

## 3. Technical Architecture

### 3.1 Domain Model

**Technology**: ClickHouse (evidence + mirror) and MariaDB (journal + caches)

**Core Entities** (implemented):

| Entity | Store | Description | Key |
|---|---|---|---|
| `identity_inputs` | ClickHouse | Identity observations from connectors — one row per changed value per source account | `(insight_tenant_id, insight_source_id, value_type, value)` |
| `persons` | MariaDB | Append-only observation journal — the source of truth for the account→person binding; every row carries `author_person_id` and `reason` | natural key ending in `created_at` (see §3.7) |
| `account_person_map` | MariaDB | Derived SCD2 cache of the binding, rebuilt from `persons` | `(tenant, source_type, source_id, account, valid_from)` |
| `operations` | MariaDB | Journal of admin operations (seed runs, future operator corrections) — request, summary, lifecycle | `id` |
| `identity.identity_persons` | ClickHouse | Read-side mirror of `persons`, republished atomically by persons-sync; consumed by the dbt `resolve_person_id` macro | mirror of `persons` |

**Future entities** (designed, not built — see §3.7 future tables): `aliases` as a resolution store, `match_rules`, `unmapped`, `conflicts`, `merge_audits`, `alias_gdpr_deleted`.

**Relationships**:
- `identity_inputs` → (folded by persons-seed, ADR-0002) → `persons`
- `persons` → (deterministic rebuild) → `account_person_map`
- `persons` → (persons-sync atomic swap) → `identity.identity_persons` → (dbt `resolve_person_id`) → gold `person_id` columns
- Operator corrections (ADR-0003) → append to `persons`; payloads journaled in `operations`
- `persons.person_id` is the stable cross-domain join key (random UUIDv7, never re-derived; ADR-0002)

### 3.2 Component Model

```text
┌────────────────────────────────────────────────────────────────┐
│                     Identity Resolution                        │
│                                                                │
│  ┌────────────────┐        ┌──────────────────────────────┐   │
│  │ persons-seed   │───────▶│                              │   │
│  │ (scheduled     │appends │   persons journal (MariaDB)  │   │
│  │  3-mode fold)  │        │                              │   │
│  └────────────────┘        └──▲──────────┬────────────────┘   │
│                               │          │                    │
│  ┌─────────────────────────┐  │          │  ┌──────────────┐  │
│  │ Operator Resolution API │──┘          └─▶│ persons-sync │  │
│  │ bind/merge/detach/      │appends  reads  │ (CH mirror)  │  │
│  │ exclude + review queue  │                └──────────────┘  │
│  └─────────────────────────┘                                  │
│                                                                │
│  ┌─────────────────────────┐   ┌───────────────────────────┐  │
│  │ Identity Read API       │   │ MatchingEngine (future)   │  │
│  │ /v1/profiles etc.       │   │ match_rules + proposals   │  │
│  └─────────────────────────┘   └───────────────────────────┘  │
└────────────────────────────────────────────────────────────────┘
```

#### PersonsSeed

- [x] `p1` - **ID**: `cpt-insightspec-ir-component-persons-seed`

##### Why this component exists

Folds new connector observations from `identity_inputs` into the `persons` journal. Without it, no account is ever bound to a person. Implemented as the `seed` subcommand of the Rust `identity-resolution` service (issue #1690), run as a scheduled job by the umbrella chart; it supersedes the original one-shot Python seed (kept under `seed/` for history).

##### Responsibility scope

Implemented behaviour (groups accounts by normalized current e-mail, then resolves each group in priority order):

- Reads `identity_inputs` — the **full evidence set each run** (no incremental watermark yet; REC-IR-02) and currently **without a tenant predicate** (producer-side tenant ids are hashed; single-tenant deployments only — restoring the filter is a multi-tenant prerequisite). UPSERT rows carry values; DELETE rows are closure signals only. **Known gap**: by the write contract DELETE rows arrive with an empty `value`, and the current reader's non-empty filter drops them — closure/tombstone handling is inert until the reader fix ships with the manual-resolution feature. Groups observations per source account and accounts by e-mail.
- Branch 1 — reuse: a group containing an already-bound account reuses that binding. **Known gap**: if the group's accounts are bound to *different* persons, the whole group currently collapses onto the first binding (with `known_binding_conflicts` counted and logged) — which can silently re-bind the other accounts.
- Branch 2 — `LinkedByEmail`: an unbound group whose e-mail already maps to an existing person is linked to that person automatically.
- Branch 3 — mint: a new person for a group with a new e-mail (at least one active profile); groups with no e-mail, or wholly closed, are skipped.
- Writes observations via `INSERT IGNORE` (idempotent for re-emitted observations under the natural key, which includes `created_at` taken from `_synced_at`); rebuilds `account_person_map` atomically.
- Guards destructive/suspicious runs (empty input, foreign tenant) with an explicit `--force` override; journals every run in `operations` (queued → running → completed/failed with summary counters).

Hardening shipped with the manual-resolution feature (required by ADR-0003; not yet implemented):

- Divergent groups: respect each account's own binding instead of collapsing — an operator-authored binding is authoritative and must never be re-bound by the group.
- Author-aware classification: divergence explained by an operator-authored binding is a resolved state (silent); all-seed divergence is surfaced for review. Requires the bindings loader to return the binding author alongside `person_id` (today it returns only `person_id`).
- Ambiguity handling: an e-mail mapped to more than one person stops auto-linking new accounts (surfaced instead).

##### Responsibility boundaries

- NEVER re-derives the binding of a consistently-bound account; after the hardening above, never re-binds any bound account.
- NEVER merges two existing persons.
- Does NOT serve API requests — that is the read API and the operator resolution API.

##### Related components (by ID)

- `cpt-insightspec-ir-component-persons-sync` — republishes what the seed wrote
- `cpt-insightspec-ir-component-operator-resolution-api` — resolves what the seed quarantined

---

#### PersonsSync

- [x] `p1` - **ID**: `cpt-insightspec-ir-component-persons-sync`

##### Why this component exists

The analytical pipeline (dbt) cannot read MariaDB directly at build time; it needs the journal in ClickHouse. persons-sync republishes `persons` into `identity.identity_persons` so the `resolve_person_id` macro can resolve `person_id` for every gold build.

##### Responsibility scope

- Mirrors the full `persons` journal into ClickHouse as an atomic snapshot swap (`EXCHANGE TABLES`), so readers never observe a partial mirror.
- Runs inside the identity-resolution service as a background worker.

##### Responsibility boundaries

- Read-only with respect to `persons`; owns only the mirror table.
- Does NOT transform or filter — resolution semantics live solely in the dbt macro.

##### Related components (by ID)

- `cpt-insightspec-ir-component-persons-seed` — produces the journal being mirrored

---

#### IdentityReadApi

- [x] `p1` - **ID**: `cpt-insightspec-ir-component-identity-read-api`

##### Why this component exists

Backend consumers (gateway, analytics API) need person lookups at request time: profile by e-mail or account, visible persons, org subchart. Specified in detail in the component spec (`docs/components/backend/identity-resolution/identity/`).

##### Responsibility scope

- `POST /v1/profiles`, `POST /v1/visible-persons` and related read endpoints over the `persons` journal (latest-per-source semantics; single-result invariant with `422 ambiguous_profile` on violation).
- Treats `persons` as an append-only event log; performs no writes.

##### Responsibility boundaries

- Does NOT write to the journal — writes come only from the seed and the operator resolution API.
- Does NOT expose analytical/bulk resolution — that is the dbt build path.

##### Related components (by ID)

- `cpt-insightspec-ir-component-operator-resolution-api` — shares the service process; owns the write path

---

#### OperatorResolutionApi

- [ ] `p1` - **ID**: `cpt-insightspec-ir-component-operator-resolution-api`

##### Why this component exists

Automatic binding can err in both directions (under-merge, over-merge), and the product offers no supported way to change a binding once automation has written it. This component adds that capability: operator correction verbs whose effects are appended to the journal and survive every re-run (ADR-0003). Reviewed design with scenarios: constructorfabric/insight#2180. Planned for the current iteration; endpoint-level behaviour will be specified at FEATURE level.

##### Responsibility scope

- Write verbs, each appending binding observations authored by the operator: `bind` (single and bulk, addressable by account or by unambiguous observed value; pre-registration of not-yet-observed accounts allowed), `merge` (explicit surviving person), `detach` (mints a new person), `exclude` (binds to the reserved excluded-person sentinel).
- Read surface: the derived review queue — `identity_inputs` evidence joined with current `persons` bindings: accounts pending a decision, contested-binding groups, and no-evidence accounts, each with candidates; resolution-rate shares (bound / pending / no-evidence / excluded — the operator-visible match rate); per-account binding history (explain); per-person account listing (matching table).
- Journals every call in `operations` (actor, request, comment); idempotency is decision-aware (§3.3) — a correction is a reported no-op only when an identical operator decision is already recorded; a bind-to-self over an automation-authored binding is the confirm act and appends the operator row.
- Ships together with two enablers outside the service: the dbt resolver upgrade (§4.4 — corrections are `value_type='id'` bindings and must reach gold) and the persons-seed hardening (see PersonsSeed) that protects operator bindings in divergent groups.

##### Responsibility boundaries

- Never updates or deletes journal rows — corrections are new observations; undo is a counter-action.
- Does NOT create or edit person-domain golden-record attributes — only the account→person binding.
- Does NOT auto-apply any suggestion — every write is an explicit operator act (no-silent-merge invariant).

##### Related components (by ID)

- `cpt-insightspec-ir-component-persons-seed` — its known-account rule makes these decisions durable
- `cpt-insightspec-ir-component-identity-read-api` — same service; read counterpart

---

#### MatchingEngine (future)

- [ ] `p3` - **ID**: `cpt-insightspec-ir-component-matching-engine`

##### Why this component exists

Future rule-driven matcher producing confidence-scored merge **proposals** for operator review (never auto-applied — no-silent-merge invariant; see §4.2 for the phased rule catalogue). Not built; requirements #1765/#796.

##### Responsibility scope

- Evaluates configurable `match_rules` (exact, normalization, cross-system, fuzzy) against journal evidence; emits proposals ordered by confidence.
- Consumes the operator journal as labeled training data; must respect operator decisions as overriding constraints.

##### Responsibility boundaries

- MUST NOT write bindings — proposals only; acceptance is an operator act through the operator resolution API.
- Fuzzy rules never auto-link regardless of score.

##### Related components (by ID)

- `cpt-insightspec-ir-component-operator-resolution-api` — the only path by which a proposal becomes a binding

> Retired v2.0 components (IDs no longer defined): the BootstrapJob is superseded by PersonsSeed; the ResolutionService is split into IdentityReadApi and OperatorResolutionApi; the ConflictDetector is absorbed into the PersonsSeed author-classification and the derived review queue.

---

### 3.3 API Contracts

- [ ] `p2` - **ID**: `cpt-insightspec-ir-interface-api-v2`

- **Technology**: REST / HTTP JSON, served by the Rust `identity-resolution` service

**Evidence write contract**: connectors write observations to `identity_inputs` per `cpt-ir-contract-bootstrap-inputs` (PRD §7.2); schema in §3.7.

**Implemented read surface** (contract details in the component spec, `docs/components/backend/identity-resolution/identity/specs/`):

| Method | Path | Description | Status |
|---|---|---|---|
| `POST` | `/v1/profiles` | Person profile lookup by e-mail / account; single-result invariant, `422 ambiguous_profile` on violation | implemented |
| `POST` | `/v1/visible-persons` | Visibility-scoped person listing | implemented |
| `GET` | `/v1/persons-seed`, `/v1/persons-seed/{id}` | Inspect seed runs recorded in the `operations` journal (list / by id); there is no HTTP seed trigger — runs are scheduled | implemented |
| `GET` | `/health`, `/healthz` | Liveness/readiness | implemented |

**Planned operator resolution surface** (ADR-0003; reviewed design with request/response shapes and scenarios in constructorfabric/insight#2180; exact contracts to be fixed at FEATURE level):

| Method | Path (working) | Description | Status |
|---|---|---|---|
| `POST` | `/v1/resolution/bind` | Bind account(s) to a person — single or bulk; addressable by account or unambiguous observed value; pre-registration allowed; also serves as "confirm" | planned |
| `POST` | `/v1/resolution/merge` | Merge two persons; operator names the surviving person explicitly | planned |
| `POST` | `/v1/resolution/detach` | Detach an account into a freshly minted person | planned |
| `POST` | `/v1/resolution/exclude` | Mark an account as not-a-person (bot/service); binds to the excluded sentinel | planned |
| `GET` | `/v1/resolution/attention` | Review queue derived from `identity_inputs` evidence joined with current `persons` bindings; evidence is folded per account over UPSERT/DELETE events, so accounts whose latest event is a closure drop out. Items: accounts pending a decision, contested-binding groups, no-evidence accounts — with candidates, counts, and resolution-rate shares (match rate) | planned |
| `GET` | `/v1/resolution/accounts/{source}/{id}` | Binding history / explain for one account | planned |
| `GET` | `/v1/resolution/persons/{person_id}/accounts` | Matching table: every account/value bound to a person, with author of each link | planned |

All write verbs append to the `persons` journal (never update/delete), record the call in `operations`, and require an operator grant enforced through the service's existing `roles` / `person_roles` tables (per-tenant grants with author and reason; whether a dedicated identity-operator role is introduced is a rollout decision). Bulk `bind` reports per-item outcomes with machine-readable skip reasons. Idempotency is decision-aware and enforced at the API level: an item is a reported no-op only when an **identical operator decision is already recorded** (same target person AND operator-authored effective binding). A bind whose target equals the current person but whose effective binding is automation-authored is NOT a no-op — it is a confirmation and appends the operator row. Key-level dedup alone cannot provide this (the journal's natural key includes `created_at`).

**Future surface** (with the MatchingEngine): match-rule configuration and proposal review endpoints — not designed yet beyond §4.2.

---

### 3.4 Internal Dependencies

| Dependency Module | Interface Used | Purpose |
|---|---|---|
| dbt models (Bronze → Silver) | ClickHouse tables | Connectors populate `identity_inputs` via the `identity_inputs_from_history` macro applied to `fields_history` models |
| persons-seed (`seed` subcommand) | ClickHouse read + MariaDB write | Folds `identity_inputs` into `persons` per ADR-0002; scheduled by the umbrella chart; journaled in `operations` |
| persons-sync (service worker) | MariaDB read + ClickHouse write | Republishes `persons` into `identity.identity_persons` via atomic swap |
| dbt `resolve_person_id` macro | ClickHouse mirror | The single place resolution semantics live for analytics; every gold model resolves `person_id` through it at build time |
| Person domain | `persons` observations (read) | Person domain builds its golden record from this domain's journal |

**Dependency Rules**:
- No circular dependencies between identity-resolution and person domains
- Identity resolution writes only to its own tables (`persons`, `account_person_map`, `operations`, the mirror)
- Person domain does not depend on identity-resolution internals; it consumes `persons` observations read-only
- Analytics never reads MariaDB directly — only the ClickHouse mirror through the macro

---

### 3.5 External Dependencies

#### ClickHouse (Evidence + Mirror)

| Aspect | Value |
|---|---|
| Tables | `identity_inputs` (MergeTree, connector-written), `identity.identity_persons` (mirror, swapped atomically), legacy `identity.aliases` |
| Version | 24.x+ (for `generateUUIDv7()` support) |
| Access | Read by persons-seed; written by dbt (inputs) and persons-sync (mirror) |
| Connection | HTTP interface / native protocol |

#### MariaDB (Journal)

| Aspect | Value |
|---|---|
| Database | `identity` — `persons`, `account_person_map`, `operations` (+ service-owned auxiliary tables) |
| Schema ownership | `identity-resolution` service via embedded SeaORM `Migrator` (ADR-0006) |
| Access | Written by persons-seed and (planned) the operator resolution API; read by the service APIs and persons-sync |

#### Kubernetes (Orchestration)

| Aspect | Value |
|---|---|
| Purpose | Schedules persons-seed runs (umbrella chart); hosts the identity-resolution Deployment |
| Note | Argo-orchestrated BootstrapJob was the v2.0 plan and is not used; a future matcher may reintroduce workflow orchestration |

---

### 3.6 Interactions & Sequences

#### Seed Run (three-mode fold)

**ID**: `cpt-insightspec-ir-seq-seed-run`

```mermaid
sequenceDiagram
    participant Sched as Scheduler (umbrella chart)
    participant Seed as persons-seed
    participant BI as identity_inputs (CH)
    participant P as persons (MariaDB)
    participant M as account_person_map (MariaDB)
    participant OPS as operations (MariaDB)

    Sched ->> Seed: run(tenant)
    Seed ->> OPS: INSERT run (queued -> running)
    Seed ->> BI: SELECT UPSERT rows, non-empty values
    BI -->> Seed: observations grouped per source account
    Seed ->> P: load known bindings + known e-mails

    loop For each e-mail group of accounts
        alt Group has a bound account
            Seed ->> P: reuse binding; INSERT IGNORE observations
            Note over Seed: divergent bindings in one group currently collapse<br/>onto the first (known_binding_conflicts++) - hardening<br/>with manual resolution makes per-account bindings win
        else Unbound, e-mail maps to an existing person
            Seed ->> P: LinkedByEmail - link group to that person
        else Unbound, new e-mail (active profile)
            Seed ->> P: mint person_id, INSERT observations (group shares person)
        else No e-mail / all closed
            Seed ->> Seed: skip
        end
    end

    Seed ->> M: transactional rebuild (tenant-scoped DELETE + INSERT, same txn)
    Seed ->> OPS: UPDATE run (completed + summary counters)
```

---

#### Operator Correction (planned — ADR-0003)

**ID**: `cpt-insightspec-ir-seq-operator-correction`

```mermaid
sequenceDiagram
    participant Op as Operator
    participant API as OperatorResolutionApi
    participant P as persons (MariaDB)
    participant OPS as operations (MariaDB)
    participant M as account_person_map (MariaDB)

    Op ->> API: POST /v1/resolution/{bind|merge|detach|exclude}
    API ->> P: validate targets (accounts, persons, tenant)
    API ->> OPS: INSERT operation (actor, request, comment)
    API ->> P: APPEND binding observation(s), author = operator UUID
    Note over P: never UPDATE/DELETE - corrections are new facts
    API ->> M: rebuild affected bindings (atomic)
    API -->> Op: result (e.g. new_person_id for detach; per-item report for bulk bind)
    Note over P: next persons-sync publish + next gold build<br/>re-attribute all history to the corrected persons
```

---

#### Build-Time Resolution (analytics read path)

**ID**: `cpt-insightspec-ir-seq-build-resolution`

```mermaid
sequenceDiagram
    participant Sync as persons-sync
    participant P as persons (MariaDB)
    participant MIR as identity.identity_persons (CH)
    participant DBT as dbt build (resolve_person_id)
    participant G as gold tables

    Sync ->> P: read full journal
    Sync ->> MIR: EXCHANGE TABLES (atomic snapshot swap)
    DBT ->> MIR: v1: latest e-mail observation per e-mail -> person_id
    Note over DBT: upgrade with manual resolution: account-first<br/>(latest value_type='id' binding per source account),<br/>e-mail fallback for facts without an account,<br/>contested e-mail -> NULL
    DBT ->> G: person_id column (honest NULL when unresolved;<br/>excluded sentinel maps to NULL)
    Note over G: person_id is recomputed on every build - corrections<br/>re-attribute history once the account-first upgrade lands
```

---

### 3.7 Database Schemas & Tables

- [ ] `p3` - **ID**: `cpt-insightspec-ir-db-schemas`

Implemented tables: `identity_inputs` (ClickHouse, evidence), `identity.identity_persons` (ClickHouse, persons-sync mirror of the journal), `persons` / `account_person_map` / `operations` (MariaDB, owned by the Rust `identity-resolution` service — see ADR-0002, ADR-0003, ADR-0006). The ClickHouse `aliases` table exists but is legacy (below). The remaining v2.0 tables (`match_rules`, `unmapped`, `conflicts`, `merge_audits`, `alias_gdpr_deleted`) were never built — see "Future tables" at the end of this section. Naming follows PR #55 conventions. For ClickHouse tables: no Nullable unless semantically required; use empty string (`''`) or zero sentinel (`'1970-01-01'`) instead.

#### Table: `identity_inputs`

**ID**: `cpt-insightspec-ir-dbtable-identity-inputs`

Alias observations from connectors. Each row represents one changed alias value from one source.

**Population mechanism**: Connectors populate this table via dbt models using the shared `identity_inputs_from_history` macro. Each connector declares:
- `identity_fields` — mapping of source field names to alias types (e.g., `workEmail → email`, `employeeNumber → employee_id`)
- `deactivation_condition` — SQL expression that detects entity deactivation (e.g., `field_name = 'status' AND new_value = 'Inactive'`), which emits DELETE rows for all identity fields

The macro reads from the connector's `fields_history` model (field-level change log from snapshots) and produces:
- **UPSERT** rows when an identity-relevant field changes
- **DELETE** rows (with empty `value`) when the deactivation condition is met

Per-connector staging tables (e.g., `staging.bamboohr__identity_inputs`) are unified into a single `identity.identity_inputs` view via `union_by_tag('silver:identity_inputs')`. The view stores nothing: any erasure obligation (GDPR purge) must target the physical staging tables and their upstream history, and guard dbt rebuilds — see the purge use case in the PRD.

The models are incremental (`append` strategy): each run processes only `fields_history` rows with `updated_at` newer than the last `_synced_at` in the target table.

| Column | Type | Description |
|---|---|---|
| `id` | `UUID DEFAULT generateUUIDv7()` | PK |
| `insight_tenant_id` | `UUID` | Tenant isolation |
| `insight_source_id` | `UUID` | Source system ID from connector config |
| `insight_source_type` | `LowCardinality(String)` | Source type (e.g., `bamboohr`, `gitlab`, `zoom`) |
| `source_account_id` | `String` | Raw account ID from the external system |
| `value_type` | `LowCardinality(String)` | `id`, `email`, `username`, `employee_id`, `display_name`, `platform_id` |
| `value` | `String` | The alias value as received from source |
| `value_field_name` | `String` | Fully-qualified source field: `bronze_{descriptor.name}.{table}.{field}[.json_path]` |
| `operation_type` | `LowCardinality(String)` | `UPSERT` or `DELETE` |
| `effective_from` | `DateTime64(3, 'UTC')` | When this alias became effective (optional; `'1970-01-01'` if unknown) |
| `effective_to` | `DateTime64(3, 'UTC')` | When this alias ceased (optional; `'1970-01-01'` if still active) |
| `_synced_at` | `DateTime64(3, 'UTC')` | Ingestion timestamp |
| `created_at` | `DateTime64(3, 'UTC')` | Row creation time |

**PK**: `id`

**ORDER BY**: `(insight_tenant_id, insight_source_id, value_type, value, _synced_at)`

**Engine**: `MergeTree`

**Normalization rules**: applied by consumers at read time, never at write time (raw values are preserved in this table). The seed groups by lowercased e-mail; the analytics macro applies `lower(trim(...))` to both join sides; the future matcher defines its own per-rule normalization (§4.2).

**Example**:

| insight_tenant_id | insight_source_id | insight_source_type | source_account_id | value_type | value | value_field_name | operation_type |
|---|---|---|---|---|---|---|---|
| `t-001` | `src-bamboo` | `bamboohr` | `E123` | `email` | `anna.ivanova@acme.com` | `bronze_bamboohr.employees.workEmail` | `UPSERT` |
| `t-001` | `src-bamboo` | `bamboohr` | `E123` | `employee_id` | `E123` | `bronze_bamboohr.employees.id` | `UPSERT` |
| `t-001` | `src-gitlab` | `gitlab` | `42` | `email` | `anna.ivanova@acme.com` | `bronze_gitlab.users.email` | `UPSERT` |
| `t-001` | `src-gitlab` | `gitlab` | `42` | `username` | `aivanova` | `bronze_gitlab.users.username` | `UPSERT` |

---

#### Table: `aliases` (LEGACY)

**ID**: `cpt-insightspec-ir-dbtable-aliases`

> **Status: legacy.** The table physically exists in ClickHouse and is populated by dbt seed models only (always with `confidence = 1.0`), but it is **not consumed by the resolution path** — analytics resolves through the `identity.identity_persons` mirror and the `resolve_person_id` macro, and the service resolves through `persons`. Retirement candidate; retained here for reference until the future MatchingEngine decides whether to reuse or replace it.

Resolved alias-to-person mapping. Each row links one `(value_type, value)` from one source to one person.

| Column | Type | Description |
|---|---|---|
| `id` | `UUID DEFAULT generateUUIDv7()` | PK |
| `insight_tenant_id` | `UUID` | Tenant isolation |
| `person_id` | `UUID` | Logical FK → `persons.person_id` |
| `value_type` | `LowCardinality(String)` | `id`, `email`, `username`, `employee_id`, `display_name`, `platform_id` |
| `value` | `String` | Normalized alias value |
| `value_field_name` | `String` | Fully-qualified source field path (from identity_inputs) |
| `insight_source_id` | `UUID` | Source system that provided this alias |
| `insight_source_type` | `LowCardinality(String)` | Source type |
| `source_account_id` | `String` | Raw account ID in the source system |
| `confidence` | `Float32` | Match confidence (1.0 = exact) |
| `is_active` | `UInt8` | 1 = active, 0 = deactivated |
| `effective_from` | `DateTime64(3, 'UTC')` | When this alias mapping became effective |
| `effective_to` | `DateTime64(3, 'UTC')` | When this alias mapping ceased (`'1970-01-01'` = current) |
| `first_observed_at` | `DateTime64(3, 'UTC')` | First time this alias was seen from this source |
| `last_observed_at` | `DateTime64(3, 'UTC')` | Last time this alias was confirmed from this source |
| `created_at` | `DateTime64(3, 'UTC')` | Row creation time |
| `updated_at` | `DateTime64(3, 'UTC')` | Last modification time |
| `is_deleted` | `UInt8` | Soft-delete flag (0 = active, 1 = deleted) |

**PK**: `id`

**ORDER BY**: `(insight_tenant_id, value_type, value, insight_source_id, id)`

**Engine**: `ReplacingMergeTree(updated_at)`

**Constraints**:
- Logical uniqueness: one active alias per `(insight_tenant_id, value_type, value, insight_source_id)` at any time — enforced at application level since ClickHouse lacks unique constraints
- `is_deleted = 1` rows are excluded from resolution lookups

**Example**:

| insight_tenant_id | person_id | value_type | value | insight_source_type | confidence | is_active |
|---|---|---|---|---|---|---|
| `t-001` | `p-1001` | `email` | `anna.ivanova@acme.com` | `bamboohr` | 1.0 | 1 |
| `t-001` | `p-1001` | `employee_id` | `E123` | `bamboohr` | 1.0 | 1 |
| `t-001` | `p-1001` | `username` | `aivanova` | `gitlab` | 0.95 | 1 |

---

#### Table: `persons` (MariaDB)

**ID**: `cpt-insightspec-ir-dbtable-persons-mariadb`

Identity-attribute observation history for persons, stored in MariaDB. Each row represents one observed value of one named attribute for one source-account at one moment in time — an SCD-style append-only log. Every connector emits a `value_type='id'` observation (value = `source_account_id`) in addition to its attribute observations, which makes `persons` the **authoritative source of truth** for the account→person binding (see ADR-0002). Attribute value types include `id` (binding anchor), `email`, `display_name`, `employee_id`, `platform_id`, and is extensible to any custom field (e.g., `functional_team`).

**Database**: MariaDB, database `identity` — dedicated to identity-resolution-domain tables, reached via the service's `database_url` configuration. The service does not assume co-location with any other MariaDB database; any other service owning MariaDB tables configures its own connection independently. Each backend service owns and applies its own schema — see [ADR-0006](../../ingestion/specs/ADR/0006-service-owned-migrations.md).

**DDL**: SeaORM migrations in `src/backend/services/identity-resolution/src/migration/` (applied by the service's `migrate` subcommand). The shape below reflects the migration chain as of `m20260724_000014` — notably migration 004, which (a) moved the natural-key UNIQUE from `value_hash` to `created_at` so that legitimate value re-transitions (Active → Inactive → Active) are recordable, and (b) switched `value_id` to case-insensitive collation. The migrations are authoritative; on divergence, trust them over this table.

##### Columns

| Column | Type | Description |
|---|---|---|
| `id` | `BIGINT UNSIGNED AUTO_INCREMENT` | PK (row identifier for operator references) |
| `value_type` | `VARCHAR(50) NOT NULL` | Attribute kind — canonical: `id`, `email`, `username`, `display_name`. Known custom: `employee_id`, `platform_id`, etc. (free-form, extensible) |
| `insight_source_type` | `VARCHAR(30) NOT NULL` | Source system: `bamboohr`, `zoom`, `cursor`, `claude_admin`, `gitlab`, etc. Tiny tier — connector keys are short, owned vocabulary (longest today is `claude_enterprise` = 17 chars) |
| `insight_source_id` | `BINARY(16) NOT NULL` | Connector instance UUID (temporary: sipHash128 from Bronze string `source_id` until `sources` table exists — see REC-IR-04) |
| `insight_tenant_id` | `BINARY(16) NOT NULL` | Tenant UUID (temporary: sipHash128 from Bronze string `tenant_id` until `tenants` table exists — see REC-IR-04) |
| `value_id` | `VARCHAR(320) COLLATE utf8mb4_unicode_ci NULL` | Value for `value_type IN ('id', 'email', 'username')`. Case-insensitive comparison (migration 004; see component ADR-0011); hot-path lookup target. Size 320 covers RFC 5321/5322 email maximum (64 local + `@` + 255 domain) |
| `value_full_text` | `VARCHAR(512) COLLATE utf8mb4_unicode_ci NULL` | Value for `value_type='display_name'`. Case- and accent-insensitive collation for operator search; leaves room for future FULLTEXT index |
| `value` | `TEXT NULL` | Catch-all value for any other `value_type` (e.g., `employee_id`, `platform_id`, `functional_team`, custom attributes). Not directly indexed |
| `value_effective` | `TEXT GENERATED ALWAYS AS (COALESCE(value_id, value_full_text, value)) STORED` | Human-readable coalesce of the three value columns; **not indexed** (display only). Use it from SELECTs when you want the actual value without knowing the routing rules |
| `value_hash` | `CHAR(64) COLLATE ascii_bin GENERATED ALWAYS AS (SHA2(COALESCE(value_id, value_full_text, value), 256)) STORED` | SHA-256 hex of the routed value. Fixed-width, byte-compared. **No longer part of the natural-key UNIQUE** (migration 004 replaced it with `created_at` — the original hash key wrongly collapsed value re-transitions); retained as a stable value digest |
| `person_id` | `BINARY(16) NOT NULL` | Person UUID (random UUIDv7). Stable; never re-derived from attribute values. See ADR-0002 |
| `author_person_id` | `BINARY(16) NOT NULL` | Person UUID of who/what made this change. Sentinel `00000000-0000-0000-0000-000000000000` = auto-minted by seed; real operator UUIDs for operator corrections (ADR-0003). This field is load-bearing: the seed and the review queue classify binding divergence by author (operator-authored = resolved state) |
| `reason` | `TEXT NULL` (migration 009) | Machine-readable change-reason code. NULL/empty for normal seed observations; `pending-iresolution` is reserved vocabulary from ADR-0002 for the deferred quarantine flow (not emitted by the current seed); operator corrections will use `operator-bind` / `operator-merge` / `operator-detach` / `operator-exclude` (ADR-0003; free-text commentary lives in the `operations` journal) |
| `created_at` | `DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6))` (migration 009) | When this record was inserted (microsecond precision, UTC by convention). The seed sets it from each observation's `identity_inputs._synced_at`, not from the seed wall-clock, so chronology in `persons` reflects when the source actually saw each value. Part of the natural-key UNIQUE (migration 004) — see the index note below for the uniqueness obligations this puts on writers |

**Hardcoded routing by `value_type`** (applied in seed + dbt macro):

| `value_type` values | target column |
|---|---|
| `id`, `email`, `username` | `value_id` |
| `display_name` | `value_full_text` |
| anything else | `value` |

Exactly one of `(value_id, value_full_text, value)` is populated per normal row; the other two are NULL. All-three-NULL is reserved for future "attribute unset at source" events (not emitted by the initial seed).

**Reserved for the matcher iteration**: proposal `confidence` and `evidence` receive first-class storage when the MatchingEngine lands (reserved column names). The journal deliberately carries no dead columns for them now — an additive MariaDB migration is cheap at that point, and operator provenance (`author_person_id`, `reason`, the `operations` payload) already accumulates the training signal.

**UUID representation**: all UUID columns are stored as `BINARY(16)` (SeaORM `.uuid()` default on MariaDB, matches `analytics` convention). Python clients must pass **`uuid.UUID.bytes`** (the 16-byte raw form) — passing the `uuid.UUID` object directly makes the driver fall back to `str(UUID)` (36 chars) which `BINARY(16)` silently truncates to ASCII, corrupting the column. For human-readable reads in SQL use `CAST(col AS UUID)` (MariaDB 10.7+) or build the textual form from `HEX(col)`. Note: MySQL 8's `BIN_TO_UUID()` is **not** available in MariaDB.

**Primary key**: `id` (auto-increment integer — MariaDB convention for append-only observation history).

**Indexes**:
- `idx_value_id (insight_tenant_id, value_type, value_id)` — hot-path reverse lookup: find person(s) by id / email. Full column indexed (320 × 4 bytes utf8mb4 = 1280 bytes, well under InnoDB's 3072-byte key budget)
- `idx_value_full_text (insight_tenant_id, value_type, value_full_text)` — secondary lookup by display name. Full column indexed (512 × 4 bytes = 2048 bytes, still within budget). Collation `utf8mb4_unicode_ci` enables case/accent-insensitive search
- `idx_person_id (person_id)` — list all attributes for a person
- `idx_tenant_person (insight_tenant_id, person_id)` — tenant-scoped person lookup
- `idx_source (insight_source_type, insight_source_id)` — filter by source system + instance
- `uq_person_observation (insight_tenant_id, person_id, insight_source_type, insight_source_id, value_type, created_at)` UNIQUE — the natural observation key as of migration 004. `created_at` (taken from the observation's `identity_inputs._synced_at` for seed writes) disambiguates repeated observations: re-emission of the same observation at the same `created_at` collapses via `INSERT IGNORE` (seed re-run idempotency), while a genuine later re-observation of the same value is a new history row. Two obligations follow for writers: (a) the key does **not** deduplicate a re-applied operator correction (its `created_at` is new) — correction idempotency is decision-aware at the API level (§3.3); (b) the key contains **no account discriminator** — two `value_type='id'` observations for two different accounts of the same source, bound to the same person at the same `created_at`, collide and `INSERT IGNORE` silently drops one. Every write path MUST therefore guarantee per-account timestamp uniqueness within the key: the correction path allocates strictly increasing `DATETIME(6)` timestamps per affected row within an operation (bulk merge/bind included), and the seed must disambiguate accounts of one source resolving to one person at the same `_synced_at` (see the seed write step). Extending the key with an account discriminator is a candidate follow-up migration

##### Semantics — append-only observation history (SCD-style)

`persons` is **append-only**. A change to a person's attribute does **not** update an existing row; it inserts a new row with a later `created_at`. The "current" value of any field is the row with the latest `created_at` for that `(insight_tenant_id, person_id, value_type)` triple (or, for multi-valued fields, the latest non-empty row per source).

Field-change example for a single person (`p-1001`) — the populated value column depends on `value_type` per the hardcoded routing:

| id | value_type | value_id | value_full_text | value | insight_source_type | created_at |
|---|---|---|---|---|---|---|
| 1 | `id` | `bamboo-emp-1001` | NULL | NULL | `bamboohr` | 2026-04-01 |
| 2 | `email` | `anna.ivanova@acme.com` | NULL | NULL | `bamboohr` | 2026-04-01 |
| 5 | `display_name` | NULL | `Anna Ivanova` | NULL | `bamboohr` | 2026-04-01 |
| 8 | `employee_id` | NULL | NULL | `CKSGP0042` | `bamboohr` | 2026-04-01 |
| 120 | `display_name` | NULL | `Anna Ivanova-Petrova` | NULL | `bamboohr` | 2026-07-15 |

Row 120 supersedes row 5 as the current `display_name` for person `p-1001` (latest by `created_at`); row 5 is retained as history. Row 1 (the `value_type='id'` binding) is emitted automatically by the connector's dbt macro on every activity — on first sync it anchors the account→person binding; on subsequent syncs it is deduped by the UNIQUE key.

---

#### Table: `account_person_map` (MariaDB)

**ID**: `cpt-insightspec-ir-dbtable-account-person-map`

**SCD2 materialized cache** of the source-account → `person_id` binding, derived deterministically from `persons` rows where `value_type='id'`. Never the source of truth; rebuilt from scratch at the end of every seed run (and by future operator flows). Exists purely for fast lookup and temporal "as of date T" queries — equivalent to a window-function scan over `persons.value_type='id'`, but O(1)–O(rows-in-tenant) instead of O(observations-in-tenant).

**Database**: MariaDB, database `identity` (same as `persons`). Defined by a SeaORM migration in `src/backend/services/identity-resolution/src/migration/` (applied by the service's `migrate` subcommand).

##### Columns

| Column | Type | Description |
|---|---|---|
| `insight_tenant_id` | `BINARY(16) NOT NULL` | Tenant UUID |
| `insight_source_type` | `VARCHAR(30) NOT NULL` | Source system: `bamboohr`, `zoom`, etc. Tiny tier (see glossary §11) |
| `insight_source_id` | `BINARY(16) NOT NULL` | Connector instance UUID |
| `source_account_id` | `VARCHAR(320) NOT NULL` | Source-native account identifier (same type as `persons.value_id`, same domain) |
| `person_id` | `BINARY(16) NOT NULL` | Person UUID (random UUIDv7); derived from `persons.person_id` of the opening observation |
| `author_person_id` | `BINARY(16) NOT NULL` | Forwarded from the `persons` observation. Sentinel `00000000-0000-0000-0000-000000000000` = auto-minted by seed |
| `reason` | `VARCHAR(50) NOT NULL` | `initial-bootstrap` \| `new-account` \| `operator-merge` \| ... — forwarded from the `persons` observation |
| `valid_from` | `DATETIME(6) NOT NULL` (migration 014) | When this binding became current (microsecond precision; = `created_at` of the opening `persons` observation). Sub-second precision is required because `valid_from` is part of the PRIMARY KEY — second-level resolution would risk PK collisions for closely-spaced events |
| `valid_to` | `DATETIME(6) NULL` (migration 014) | When this binding ended (= next observation's `created_at`). `NULL` = currently active binding |

**Primary key**: `(insight_tenant_id, insight_source_type, insight_source_id, source_account_id, valid_from)` — one row per historical binding period. An account with N historical bindings has N rows; the latest has `valid_to = NULL`.

**Indexes**:
- `idx_current (insight_tenant_id, insight_source_type, insight_source_id, source_account_id, valid_to)` — fast "current binding" lookup via `WHERE valid_to IS NULL`
- `idx_person_id (person_id)` — list all accounts currently bound to a person
- `idx_tenant_person (insight_tenant_id, person_id)` — tenant-scoped person accounts
- `idx_valid_from (insight_tenant_id, valid_from)` — "bindings changed in date range" queries for dashboards

##### Semantics

- `persons` is the **source of truth**; `account_person_map` is **derived**. Drift is impossible by construction because rebuild re-derives every row from `persons`.
- **"Current binding"**: `WHERE valid_to IS NULL`.
- **"Binding as of date T"**: `WHERE valid_from <= T AND (valid_to > T OR valid_to IS NULL)` — one row per source-account, O(log N) range lookup.
- **"Full history of an account"**: all rows with the account's PK tuple, ordered by `valid_from`.
- **Rebuild** (at the end of every seed run + every future operator flow) — **transactional, tenant-scoped**: within the same transaction that applies the observations, the seed issues `DELETE FROM account_person_map WHERE insight_tenant_id = ?` followed by `INSERT ... SELECT ... LEAD() OVER (PARTITION BY tenant, source_type, source_id, account ORDER BY created_at)` from `persons.value_type='id'` rows. The journal write and the cache rebuild commit atomically — readers see either the pre-run or the post-run state, and the log and cache are never observably inconsistent. (An earlier design described a `RENAME TABLE` two-table swap; the implemented mechanism is the transactional delete-and-insert above.)

See ADR-0002 for the full decision record (why a derived cache instead of a second authoritative table, alternatives considered).

---

##### Seed (idempotent fold from `identity_inputs`)

**Implementation** — the seed is the `seed` subcommand of the Rust `identity-resolution` service (issue #1690), sharing the service's SeaORM models and configuration:

| Aspect | Value |
|---|---|
| Invocation | `identity-resolution seed` — one run, then exit. Scheduled by the umbrella chart; manually runnable for ad-hoc reseeds |
| Concurrency | A run-lock guarantees a single active run; a concurrent invocation exits with a warning |
| Guards | Suspicious inputs (empty `identity_inputs`, foreign-tenant universe) are refused unless the operator passes an explicit `--force`; the scheduled job itself never forces |
| Audit | Every run is journaled in `operations` (queued → running → completed/failed) with summary counters per mode, including `known_binding_conflicts` |
| History | The original one-shot Python seed (`seed/seed-persons-from-identity-input.py` + `seed-persons.sh`) performed the initial bootstrap and remains in the tree for reference; the Rust subcommand is the operative mechanism |

**Schema ownership**: the `persons` and `account_person_map` table DDL
lives inside the identity-resolution service at
`src/backend/services/identity-resolution/src/migration/`
and is applied by the service's own SeaORM migrator via the
`migrate` subcommand. See
[ADR-0006](../../ingestion/specs/ADR/0006-service-owned-migrations.md)
for the service-owned-migrations policy. The seed operates on the
already-created tables and never issues DDL; `persons` is never
deleted from or updated, and the only row deletion in the
transactional apply is the tenant-scoped rebuild of the derived
`account_person_map` cache.

**Process** (data flow executed by each seed run):

1. Read `identity.identity_inputs` (ClickHouse): UPSERT observation rows plus DELETE closure signals (never persisted). **Known gap**: DELETE rows carry an empty `value` by contract while the reader filters non-empty values only, so closure signals are currently dropped before the fold — the reader fix ships with the manual-resolution feature. **The full set each run** — no incremental watermark yet (REC-IR-02) — and currently without a tenant predicate (single-tenant deployments; multi-tenant prerequisite). Order by `_synced_at DESC` within each source-account so that the latest email observation is picked deterministically in step 5.
2. Group observations by `(insight_tenant_id, insight_source_type, insight_source_id, source_account_id)` — a "source account" = one user in one connector instance.
3. Connect to MariaDB. Load known bindings: for each source-account key, find the latest `value_type='id'` observation in `persons` and capture its `person_id`. This becomes the **known-account** set.
4. Load the current e-mail map: for each normalized e-mail (lowercased, not trimmed — ADR-0011 parity), take the **latest** `value_type='email'` observation (`created_at DESC, id DESC`) and its `person_id`, producing `normalized_email → person_id`. Latest-wins over the append-only journal; the map is empty on the very first run (initial bootstrap) and non-empty afterwards — the same code path handles both, there is no mode flag. (Contested-claim detection on this map ships with the manual-resolution hardening; see §3.2.)
5. Group accounts by normalized current e-mail; resolve each group in priority order (mirrors the .NET resolver; `domain/seed.rs::resolve_assignments`):
   - **Group with a bound account** (step 3 set): reuse that `person_id` for the whole group; no new binding decision. **Known gap**: if the group's accounts are bound to *different* persons, the group currently collapses onto the first binding (counted in `known_binding_conflicts`, logged) and can thereby silently re-derive a binding — the manual-resolution hardening replaces this with per-account binding respect and surfacing.
   - **Unbound group, e-mail present in step 4 set**: link the group to that person (`LinkedByEmail`) — the account joins an existing person automatically when the e-mail is unambiguous.
   - **Unbound group, new e-mail, at least one active profile**: mint a new `person_id` (random UUIDv7); accounts sharing the new e-mail within the run share the person.
   - **No e-mail, or all profiles closed**: skip — no binding is created. Skipped active accounts are not hidden: the review queue surfaces them from `identity_inputs` as no-evidence items awaiting an operator bind. E-mail remains the sole automatic identity anchor for this seed.

   > ADR-0002 additionally specified a quarantine mode (`reason='pending-iresolution'`) for contested e-mails; the implemented port kept .NET-parity auto-linking instead. Contested-evidence handling (no auto-link when an e-mail maps to more than one person) ships with the manual-resolution feature.
6. Write observations to `persons` via `INSERT IGNORE`. Routing rules (hardcoded in the seed's `route_value`, mirrored by the dbt macro):
   - id-like types (`id`, `email`, `username`, `employee_id`, `parent_email`, `parent_id`, `parent_person_id`) → `value_id = value`, others NULL
   - name-like types (`display_name`, `first_name`, `last_name`, `department`, `division`, `job_title`, `status`) → `value_full_text = value`, others NULL
   - otherwise → `value = value`, others NULL

   **Timestamp-uniqueness obligation (seed side)**: `created_at` comes from `_synced_at`, and the natural key has no account discriminator — two accounts of the same source resolving to one person at the same `_synced_at` would collide on their `value_type='id'` rows and `INSERT IGNORE` would silently drop one binding. The seed write path must disambiguate per account (same obligation as the operator path, §3.7 index note); until that hardening ships this is a known gap of the same family as the divergent-group collapse.

   `author_person_id` is the all-zero sentinel `00000000-0000-0000-0000-000000000000` for auto-minted bindings; the `uq_person_observation` UNIQUE key (on `created_at`, migration 004) dedupes re-runs. `created_at` is taken from each observation's `identity_inputs._synced_at`, not from the seed wall-clock, so chronology in `persons` reflects the source's view of when each value was seen — and re-runs over the same input reproduce the same keys.
7. **Rebuild `account_person_map`** from `persons.value_type='id'` observations — tenant-scoped `DELETE` + `INSERT ... SELECT ... LEAD()` inside the same transaction as the observation writes (see the account_person_map Semantics block). Drift relative to `persons` is impossible by construction.

**Re-run semantics**: idempotent on `persons` (UNIQUE key dedupe — same input reproduces the same `created_at` keys), bit-identical on `account_person_map` (deterministic rebuild from same `persons` state). Adding a new source between runs creates new accounts; each is linked by e-mail when unambiguous or minted fresh. A consistently-bound account is never re-derived; the divergent-group collapse above is the one remaining path that can override a binding, and closing it is a precondition of ADR-0003 correction durability.

**Conflict classification (ships with manual resolution)**: when an e-mail group's accounts are bound to more than one person, the seed will inspect the authors of the divergent bindings: any operator-authored binding marks the divergence as an intentional resolved state (not counted, not surfaced); all-seed divergence is counted in `known_binding_conflicts` and surfaced for review — and never reconciled by rewriting history. Prerequisite: the bindings loader must return `author_person_id` alongside `person_id` (today it returns only the person).

See [ADR-0002](ADR/0002-stable-person-id-via-persons-observations.md) for the full decision record.

**Safety / idempotency**:
- Re-running the script is **safe**: the `account_person_map` lookup keeps `person_id` stable across runs, and `INSERT IGNORE` on `persons` skips duplicates.
- Steady-state re-runs never merge two **existing** persons — no such code path exists. New accounts join an existing person only via the unambiguous e-mail link (`LinkedByEmail`); merging existing persons is an operator-only action (ADR-0003).
- The seed never issues `TRUNCATE`, `DELETE`, or `UPDATE` against `persons`. The only `DELETE` it performs is the tenant-scoped rebuild of the derived `account_person_map` cache, inside the same transaction as the observation writes. Wipe-and-reseed of `persons` is an explicit operator action outside the seed.

**Prerequisites and ordering** (end-to-end bootstrap):

1. Connector secrets applied (`./secrets/apply.sh`).
2. Kubernetes deploy (`cd deploy/gitops && make deploy ENV=<env>`) — installs Airbyte + Argo Workflows (L2 system) and the Insight umbrella chart (L3 app). The umbrella's `identity-db-init-job` Helm pre-install Job provisions the `identity` MariaDB database and grants. The identity-resolution pod then starts and applies its sea-orm migrations (including the `persons` table) at startup via `run_migrations(&db)` in `main.rs`.
3. `./src/ingestion/reconcile-connectors.sh` — registers connectors, creates Airbyte connections + per-connector CronWorkflows. (ClickHouse migrations run via the `clickhouse-migrate` Helm Hook Job on helm install/upgrade in step 2, not from a host script.)
4. Airbyte sync produces Bronze data (`./sync-all.sh` + wait).
5. dbt models run to populate `identity.identity_inputs` (`dbt run --select +identity_inputs`).
6. Seed run — the `identity-resolution seed` subcommand, scheduled by the umbrella chart (or invoked manually for ad-hoc reseeds).

---

#### Future tables (design proposals — not implemented)

The v2.0 design specified five further ClickHouse tables. **None of them exists in migrations or DDL today.** Their summaries are retained below for the future MatchingEngine iteration; their full column-level schemas live in the git history of this file (v2.0).

| Table | Purpose (v2.0 proposal) | Status |
|---|---|---|
| `match_rules` | Configurable matching rules (type, weight, phase, enablement) for the MatchingEngine | Future — with the matcher (§4.2, #1765) |
| `unmapped` | Persistent operator queue of unresolved observations with statuses and suggestions | Superseded for v1: the review queue is **derived** from the `identity_inputs` active-evidence fold (per-account UPSERT/DELETE) joined with current `persons` bindings — no status columns to drift. A persistent proposal store may return with the matcher |
| `conflicts` | Alias-level conflict records (same value claimed by two persons) | Superseded for v1: conflicts are derived from the journal and classified by binding author (ADR-0003); a persistent record may return with the matcher |
| `merge_audits` | Snapshot-before/after audit trail for merge/split | **Superseded by ADR-0003**: the append-only `persons` journal plus the `operations` journal are the audit trail; merge/split are appended reassignments, not snapshot-restores |
| `alias_gdpr_deleted` | GDPR erasure archive (v2.0 proposal) | **Rejected as designed**: archiving plaintext values contradicts hard erasure. The future purge flow erases values in place across the physical stores (including the staging tables behind the `identity_inputs` union view and their upstream history) and keeps at most value-free tombstones (keyed HMAC digests) as a re-link deny-list consulted by transformations — see the PRD purge use case and #719 |

---

## 4. Additional Context

### 4.1 Min-Propagation Algorithm (ClickHouse-Native)

> Source: `inbox/IDENTITY_RESOLUTION.md`
>
> **Status: future — not implemented.** Kept as candidate material for the matcher iteration (bulk verification / candidate generation). Note two ADR-0003 constraints on any future use: transitive auto-grouping must never write bindings (proposals only), and operator decisions in the journal override its output.

This is an **alternative implementation** of identity grouping — runs entirely in ClickHouse on `(token, rid)` pairs. It may be used for bulk initial grouping or as a verification tool to detect grouping inconsistencies alongside the future MatchingEngine.

**Input**: table of `(token, rid)` pairs where:
- `token` — a value identifying a person (username, email, work_email, etc.), mapped from `value` in `identity_inputs`
- `rid` = `cityHash64(insight_source_type, source_account_id)` — deterministic hash per source account

**Algorithm**:
1. **Initialize** — assign each `rid` its own value as group ID.
2. **Iterate** (default 20 passes):
   - For each token, find the **minimum** group ID among all rids sharing that exact token.
   - Propagate that minimum to every rid associated with that token.
3. **Converge** — all transitively connected rids share the same minimum group ID.
4. **Rank** — `dense_rank()` converts raw group IDs to sequential `profile_group_id` values.

Matching is always on **full token values** — no substring matching.

**Enrichment steps** applied before the algorithm:
1. **Manual identity pairs** — synthetic bridge records from a seed table (last resort).
2. **First-name aliases** — unidirectional seed table (e.g., `alexei` ↔ `alexey`). Applied only when the whole word matches (word boundaries, not substring).
3. **Email domain aliases** — seed table of equivalent domains. Records on any listed domain get synthetic variants for all other listed domains.

**Augmented groups step**: runs algorithm twice (full: real + synthetic; natural: real only). Only keeps synthetic records that actually bridged distinct natural groups — prevents synthetic inflation.

**Blacklist**: generic tokens (`admin`, `test`, `bot`, `root`) and usernames ≤ 3 characters are excluded.

**Data sources** (token fields derived from `identity_inputs.value_type` / `value`):

| Source (`insight_source_type`) | Token fields (`value_type`) | Notes |
|---|---|---|
| `git` | `username`, `email` | Lowercased |
| `zulip` | `display_name`, `email` | |
| `gitlab` | `username`, `email` | Multiple emails per user |
| `constructor` | `username`, `display_name`, `email` | |
| `bamboohr` | `display_name`, `email` | Dots replaced with spaces in names |
| `hubspot` | `display_name`, `email` | From users + owners tables |
| `youtrack` | `username`, `email` | |

Min-propagation and the future MatchingEngine are **complementary**: min-propagation is a bulk verification / candidate-generation tool; the matcher is the incremental proposal path. Neither writes bindings (ADR-0003).

---

### 4.2 Matching Engine Phases

> **Status: future — not implemented.** Neither the MatchingEngine nor the `match_rules` table exists; today's only automatic rule is the seed's exact-e-mail grouping (ADR-0002). This section is the design direction for the future matcher (#1765, #796). Whatever the final shape, two invariants from ADR-0003 bind it: the matcher produces **proposals, never writes**, and operator decisions in the journal override any rule.

The MatchingEngine (`cpt-insightspec-ir-component-matching-engine`) evaluates rules in three phases, stored in the `match_rules` table. Rules are ordered by `sort_order` within each phase.

**Value type vocabulary** (stored in `aliases.value_type` and `identity_inputs.value_type`). The `value_type` field is a free-form string, not an enum — the canonical list below is extensible, and connectors may emit any custom value-type on top.

**Canonical** (recognised throughout this domain and routed to indexed columns in `persons`):

| `value_type` | Description | Normalization |
|---|---|---|
| `id` | Canonical binding observation — value equals `source_account_id`. Emitted by every connector per ADR-0002. | `trim()` |
| `email` | Email address | `lower(trim())`, remove plus-tags, domain alias expansion |
| `username` | Platform username / login | `lower(trim())` |
| `display_name` | Human-readable full name | `trim()` |

**Known custom** (examples; connectors may define their own and the list is open-ended — all such values land in `persons.value` TEXT catch-all):

| `value_type` | Description | Normalization |
|---|---|---|
| `employee_id` | HR-system business identifier (e.g., BambooHR `CKSGP0002`) distinct from the connector's internal account id | `trim()` |
| `platform_id` | Platform-specific opaque identifier where distinct from `source_account_id`. If equal to `source_account_id`, use `id` instead. | `trim()` |

**Phase B1 — Deterministic (highest-confidence proposals)**:

| Rule (`condition_type`) | Confidence | Description |
|---|---|---|
| `email_exact` | 1.0 | Identical email after normalization |
| `hr_id_match` | 1.0 | Identical `employee_id` from same `insight_source_type` |
| `username_same_sys` | 0.95 | Same username within same `insight_source_type` |

**Phase B2 — Normalization & Cross-System (medium-confidence proposals)**:

| Rule (`condition_type`) | Confidence | Description |
|---|---|---|
| `email_case_norm` | 0.95 | Case-insensitive email match |
| `email_plus_tag` | 0.93 | Email match ignoring `+tag` suffix |
| `email_domain_alias` | 0.92 | Same local part, known domain alias |
| `username_cross_sys` | 0.85 | Same username across related systems (GitLab <-> GitHub <-> Jira) |
| `email_to_username` | 0.72 | Email local part matches username in another system |

**Phase B3 — Fuzzy (disabled by default; suggestions only, never eligible for bulk acceptance)**:

| Rule (`condition_type`) | Confidence | Description |
|---|---|---|
| `name_jaro_winkler` | 0.75 | Jaro-Winkler similarity >= 0.95 on `display_name` |
| `name_soundex` | 0.60 | Phonetic matching (Soundex) on `display_name` |

**Confidence bands** (order and gate proposals — per ADR-0003 no band ever writes a binding; the v2.0 auto-link semantics of this subsection are superseded):
- high (`>= 1.0` in the v2.0 rule catalogue) — high-confidence proposal; eligible for one-click / bulk acceptance by the operator
- medium (`0.50–0.99`) — suggestion proposal with the candidate person attached
- low (`< 0.50`) — surfaced for review without a candidate

Acceptance of any proposal is an explicit operator act through the operator resolution API; rejection and deferral are first-class outcomes.

**Email normalization pipeline**:
```
Input: "John.Doe+test@Acme.COM"
  1. lowercase        → "john.doe+test@acme.com"
  2. trim whitespace  → "john.doe+test@acme.com"
  3. remove plus tags → "john.doe@acme.com"
  4. domain alias     → also matches "john.doe@constructor.dev"
```

---

### 4.3 Operator Corrections (Merge / Split / Bind / Exclude)

> Journal-based per [ADR-0003](ADR/0003-operator-decisions-as-persons-observations.md); supersedes the v2.0 snapshot-based merge/split over ClickHouse `aliases`/`merge_audits`. Reviewed design with worked scenarios (S1–S10): constructorfabric/insight#2180. Planned for the current iteration.

Every correction is an **appended binding observation** in `persons`, authored by the operator's UUID with a machine-readable reason; the request and free-text comment are journaled in `operations`. Current state is the latest binding per account; nothing is ever updated or deleted. A correction writes **binding observations only**: which identity values (e-mails, usernames) belong to which account is not encoded in the journal — an account's own `value_type='id'` row does carry the account id (in `value_id`), but attribute observations reference only the source instance — so the account↔value linkage comes from the `identity_inputs` evidence, where every row carries `source_account_id`. The resolver (§4.4) and the value-addressed bulk `bind` both consume that linkage; the service reads `identity_inputs` through the same infrastructure the seed already uses.

**Merge** — "these two persons are one human": for each account of the absorbed person, append a binding to the surviving person (`reason='operator-merge'`). The operator names the survivor explicitly. The absorbed person keeps its history and simply ends up with no current accounts.

**Detach / split** — "this account belongs to a different human": append a binding to a freshly minted person (`reason='operator-detach'`). Works on any account **regardless of how the current grouping arose** — no prior merge record is required (e.g. separating accounts grouped through a shared mailbox such as `team@example.com`).

**Bind** — attach an account to a known person (`reason='operator-bind'`); single or bulk; also expresses "confirm" (bind-to-self on an account pending review — same person, now operator-authored, so the review item's condition dissolves) and pre-registration of accounts not yet observed.

**Exclude** — "not a human" (bot/CI/service): bind to the reserved excluded-person sentinel (`reason='operator-exclude'`).

**The excluded-person sentinel** (normative definition): the fixed UUID `ffffffff-ffff-ffff-ffff-ffffffffffff`. One global constant across tenants (rows carrying it remain tenant-scoped); collision-free by construction (UUIDv7 version/variant bits make this value unmintable). Consumers treat it uniformly as "no person": the resolve macro maps it to NULL (§4.4); the read API never serves it as a person — a lookup landing on it reports the account as not resolvable to a person; the person domain ignores observations bound to it when building golden records; the review queue hides accounts whose effective binding is the sentinel.

**Undo** — a counter-action: a newer operator binding that re-points the account. History retains the mistake, the fix, and both authors; the pre-correction state is always reconstructible from the journal.

**Durability**: corrections live in the same journal the seed reads, so no parallel override store exists to drift. Enablers shipping with the feature: the seed hardening that makes per-account bindings win over group collapse — closing the one path that could override an operator row; decision-aware API-level idempotency (§3.3 — a correction is a no-op only when an identical operator decision is already recorded, so confirmations still write); and unique per-row observation timestamps on the write path (§3.7 index note — the natural key has no account discriminator).

**Downstream**: `person_id` is recomputed by the resolve macro on every gold build. For corrections to re-attribute history, the macro's account-first upgrade (§4.4) is required: corrections are `value_type='id'` bindings keyed by source account, while the v1 macro resolves by e-mail only. Account-carrying facts then follow the binding directly; e-mail-keyed facts (e.g. git commits) follow through the account-derived e-mail fallback (§4.4), with contested e-mails resolving to NULL. Facts keyed by an e-mail that was never observed on any account remain NULL — re-attribution is complete for observed identities, not a promise about unknown e-mails.

---

### 4.4 Analytics Integration (mirror + resolve macro)

The implemented analytics integration has two moving parts:

**Mirror**: persons-sync republishes the MariaDB `persons` journal into ClickHouse `identity.identity_persons` as an atomic snapshot swap (`EXCHANGE TABLES`) — readers never observe a partial mirror. The mirror is a plain copy; no resolution semantics are applied at sync time.

**Resolve macro**: the dbt macro `resolve_person_id` is **deliberately the only place resolution semantics live** for analytics. Every gold model that needs a `person_id` calls it against the mirror at build time.

**v1 rule (implemented)**: the macro emits a current `(email → person_id)` map — the latest `value_type='email'` observation per normalized e-mail claims it (`LIMIT 1 BY email`, `created_at DESC, id DESC` tiebreak) — and gold models LEFT JOIN it on the fact's normalized e-mail (`resolved_person_id_join`). Unresolved facts yield an honest NULL (absent rather than guessed). Consequence: **only e-mail-keyed changes reach gold**; a correction expressed as a `value_type='id'` binding is invisible to this map.

**Account-first upgrade (ships with manual resolution — required by ADR-0003)**: the macro additionally emits an account map keyed by the **source-instance-scoped account identity** — `(insight_source_type, insight_source_id, source_account_id)` — from the latest `value_type='id'` binding per account. Source type and instance are mandatory key parts because account ids are unique only within one source instance; a truncated key would cross-link identical ids from different connectors. **Tenant is deliberately not a join column between evidence and journal**: `identity_inputs` carries a producer-side *hashed* tenant id while the journal carries the caller's tenant (see the reader note in §3.2 PersonsSeed), so an `insight_tenant_id` equijoin across the two stores would match nothing. The key above is consistent across both stores because the journal's source ids originate from the evidence rows. Tenant scoping is applied within the journal/mirror side; cross-store tenant joins become possible only after tenant-id normalization (REC-IR-04 `tenants` table) — the already-noted multi-tenant prerequisite. Facts that carry a source account resolve **account-first** on this key — this explicitly covers the **reviewer namespace**: platforms report code-review identities as a login/account id rather than an e-mail, so review facts resolve through the id-binding map without any e-mail bridge. Facts without an account (e.g. git commits, which are e-mail-keyed by nature) fall back to the e-mail map. The e-mail fallback is **account-derived**, so it follows corrections without any extra writes: `identity_inputs` carries `source_account_id` on every row, so the macro maps each e-mail to the accounts currently observing it (latest evidence per account, joined on the same source-instance-scoped key) and resolves those accounts through the id-binding map. All observing accounts on one person → the e-mail resolves to that person; more than one person (a genuinely shared value) or none → **NULL** (contested evidence is excluded, not tie-broken); the excluded-person sentinel maps to NULL. Known limit: a fact keyed by an e-mail never observed in `identity_inputs` has nothing to follow and stays NULL. Consistency: a build reads one snapshot of the evidence and one atomically-swapped mirror snapshot. Future smarts — per-source maps, as-of resolution, matcher output — keep changing only the macro body.

Consequences of this shape:

- **Retroactive re-attribution**: the macro recomputes `person_id` on every build, so once the account-first upgrade lands, operator corrections (§4.3) move complete activity histories without re-keying jobs; under v1 this held only for e-mail-keyed changes.
- **Build-scoped consistency**: a single build resolves every model against one mirror snapshot, so all gold tables of one build agree on who is who.
- **No request-path coupling**: analytics never queries MariaDB or the service; the mirror decouples build load from the journal store.

(The v2.0 pattern — a ClickHouse Dictionary or direct JOIN over the `aliases` table — is retired with that table; see §3.7 legacy note. Its normalization caveat survives as a rule for the macro: comparisons must be applied to normalised values on both sides.)

---

### 4.5 End-to-End Walkthrough: Anna Ivanova

> Source: `inbox/architecture/EXAMPLE_IDENTITY_PIPELINE.md`

**Sources**: BambooHR (employee_id: E123, email: anna.ivanova@acme.com), Active Directory (username: aivanova, email after name change: anna.smirnova@acme.com), GitHub (username: annai), GitLab (username: ivanova.anna), Jira (username: aivanova).

**Step 1 — dbt `identity_inputs_from_history` generates rows from `fields_history`**:

| `insight_source_type` | `source_account_id` | `value_type` | `value` | `value_field_name` |
|---|---|---|---|---|
| `bamboohr` | `E123` | `employee_id` | `E123` | `bronze_bamboohr.employees.id` |
| `bamboohr` | `E123` | `email` | `anna.ivanova@acme.com` | `bronze_bamboohr.employees.workEmail` |
| `ad` | `aivanova` | `username` | `aivanova` | `bronze_ad.users.sAMAccountName` |
| `ad` | `aivanova` | `email` | `anna.smirnova@acme.com` | `bronze_ad.users.mail` |
| `github` | `42` | `username` | `annai` | `bronze_github.users.login` |
| `gitlab` | `17` | `username` | `ivanova.anna` | `bronze_gitlab.users.username` |
| `jira` | `aivanova` | `username` | `aivanova` | `bronze_jira.users.name` |

**Step 2 — persons-seed folds the observations** (three-mode logic, ADR-0002):

1. BambooHR account `E123` — unknown, e-mail `anna.ivanova@acme.com` is new → mint person `p-1001`; observations appended under it.
2. AD account `aivanova` — unknown; its e-mail `anna.smirnova@acme.com` is new (the name changed) → mint separate person `p-1002`. The seed does not guess that `aivanova` is Anna — usernames are not linking evidence in v1.
3. GitHub `42`, GitLab `17` — unknown, no e-mail observed → not auto-bound; they surface in the review queue as no-evidence accounts (#1776) awaiting an operator bind.
4. Jira `aivanova` — unknown, no e-mail → skipped likewise.

**Step 3 — Operator corrects** (ADR-0003 verbs):

- `merge p-1002 → p-1001` ("AD is the same Anna after the name change") — one appended binding for `aivanova`, authored by the operator.
- `bind github:42 → p-1001`, `bind gitlab:17 → p-1001`, `bind jira:aivanova → p-1001` — or one bulk `bind` call with all three rows.

**Step 4 — What re-runs cannot break**:

The next seed run sees every one of these accounts as *known* (a binding exists) and reuses their bindings. A later account arriving with `anna.ivanova@acme.com` auto-links to `p-1001` (`LinkedByEmail` — the e-mail is unambiguous). With the contested-evidence hardening, an e-mail claimed by *two* persons would surface for review instead of linking; and a divergent-group collapse can no longer override the operator's merge of `p-1002`.

**Step 5 — Analytics**:

Under the v1 e-mail map, Anna's activity resolves per fact e-mail — the operator's account bindings for GitHub/GitLab/Jira do not reach gold yet. With the account-first macro upgrade (§4.4), all facts carrying those accounts — including all history from before the corrections — attribute to `p-1001`; git commits keep resolving via their commit e-mails.

**Result**: one person, six accounts across five sources; two appended operator decisions did the work the automation could not — durable in the journal, and visible end-to-end once the resolver upgrade lands.

---

### 4.6 End-to-End Walkthrough: Andrei Sokolov (Min-Propagation)

> Source: `inbox/IDENTITY_RESOLUTION.md`

This walkthrough demonstrates the min-propagation algorithm (§4.1) as a verification mechanism.

**Sources**: BambooHR (`source_account_id: b1`, `display_name: andrei sokolov`, `email: Andrei.Sokolov@acme.io`), Git commits (`source_account_id: c1–c3`, `username: sokol`, various personal emails), YouTrack (`source_account_id: y1`, `display_name: Andrey Sokolov`, `email: a.sokolov@acme.com`).

**Token extraction from `identity_inputs`**:

| `insight_source_type` | `source_account_id` | `value_type` | `value` (token) |
|---|---|---|---|
| `bamboohr` | `b1` | `display_name` | `andrei sokolov` |
| `bamboohr` | `b1` | `email` | `andrei.sokolov@acme.io` |
| `git` | `c1` | `username` | `sokol` |
| `git` | `c2` | `email` | `sokol@gmail.com` |
| `git` | `c3` | `email` | `a.sokolov@gmail.com` |
| `youtrack` | `y1` | `display_name` | `andrey sokolov` |
| `youtrack` | `y1` | `email` | `a.sokolov@acme.com` |

**After name alias enrichment** (`andrei` <-> `andrey`): BambooHR and YouTrack get synthetic tokens for each other's name spelling.

**After domain alias enrichment** (`gmail.com`, `acme.io`, `acme.com` grouped): Git c3 and YouTrack share `a.sokolov@gmail.com`.

**Min-propagation result**: `rid(b1)`, `rid(c1)`, `rid(c2)`, `rid(c3)`, `rid(y1)` → all converge to same minimum group ID → `profile_group_id = 1`.

**Verification**: Compare min-propagation grouping with the journal's current bindings (and, later, the MatchingEngine's proposals). If they disagree, investigate which links are missing or incorrect — as review input, never as an automatic write.

---

### 4.7 Deployment

**Hybrid storage** — ClickHouse for analytical identity tables, MariaDB for transactional identity-history (`persons`, `account_person_map`).

**Kubernetes (production)**:

| Component | Type | Resources |
|---|---|---|
| ClickHouse | StatefulSet (shared cluster) | Per cluster sizing |
| MariaDB | StatefulSet (Bitnami chart, shared cluster) | Per cluster sizing |
| identity-resolution | Deployment (horizontal scaling) — Rust `axum` service; runs the persons-sync worker | 0.5 CPU, 256 MB RAM per replica |
| identity-resolution migrate | InitContainer / one-shot Job — applies MariaDB schema via embedded SeaORM `Migrator` | 0.1 CPU, 64 MB RAM |
| persons-seed | Scheduled job (umbrella chart) running the service image with the `seed` subcommand; run-locked, guarded, journaled in `operations` | 0.25 CPU, 256 MB RAM per run |
| MatchingEngine (future, not built) | TBD with the matcher design | — |

**identity-resolution** is a stateless Rust (axum) service. It owns the MariaDB `identity` database — `persons` (observation journal), `account_person_map` (SCD2 cache rebuilt from `persons.value_type='id'`), `operations` (admin-operation journal) — with migrations applied at startup via SeaORM `Migrator` (see ADR-0006). The service does not read Bronze tables directly; observations flow in through `identity.identity_inputs` (populated by the per-connector dbt models that use the `identity_inputs_from_history` macro) and are folded into `persons` by the seed. The persons-sync worker republishes the journal to ClickHouse for analytics. Horizontal scaling via Kubernetes replicas.

**persons-seed** runs as a scheduled job (the service image with the `seed` subcommand — issue #1690): run-locked, input-guarded (`--force` for deliberate overrides), journaled in `operations`, idempotent via `INSERT IGNORE`. See ADR-0002. The original one-shot Python seed remains under `seed/` for reference.

**Operator resolution API** (planned, ADR-0003) ships inside the same service — no new deployable.

**Environment**: Kubernetes (Kind locally); Argo Workflows is not used by this domain today (it was the v2.0 plan for the BootstrapJob and may return with the matcher).

---

### 4.8 Operational Considerations

**Monitoring observables** (implemented and planned):

| Observable | Source | Description | Attention Threshold |
|---|---|---|---|
| Unresolved activity share | `identity_resolution_coverage` view (gold) | Share of activity (not of aliases) without a resolved `person_id`, per source — counts the operator decisions that would close the gap | Trend-based; investigate growth |
| `known_binding_conflicts` | Seed run summary in `operations` | Seed-authored binding divergence inside e-mail groups (operator-authored divergence excluded by classification) | > 0 new per run |
| Pending-decision count + match rate | Review queue (planned, `/v1/resolution/attention`; derived from evidence + bindings) | Accounts pending a decision, contested-binding groups, no-evidence accounts; resolution-rate shares | Backlog growth week-over-week |
| Seed run outcome | `operations` journal | Failed/refused runs (guard rejections) | Any failed run |
| Mirror freshness | persons-sync | Time since last successful `identity.identity_persons` swap | > 1 sync interval |
| Person lookup latency | identity-resolution service | p99 of `/v1/profiles` | > 50 ms |

**SLA targets**:
- Person lookup latency (service read path): < 50 ms p99
- Seed processing: < 30 min after connector sync completes
- Correction visibility in dashboards: next gold build after the correction (build-scoped snapshot)

**Capacity planning**:
- `identity_inputs`: grows linearly with connector syncs. Each sync produces O(changed_accounts * aliases_per_account) rows. TTL-based expiry recommended after processing.
- `aliases`: grows with total unique (value_type, value, source) combinations. Expected < 1M rows for 10K persons across 10 sources.
- `merge_audits`: grows with operator activity. Low volume, no TTL needed.

**Cross-domain references**:
- Golden Record Pattern — see **person domain DESIGN** (person attributes, source priority, completeness scoring)
- Org Hierarchy & SCD Type 2 — see **org-chart domain DESIGN** (org_units, person_assignments, temporal queries)

---

## 5. Implementation Recommendations

Recommendations for later implementation phases. Not blocking for current scope.

### REC-IR-01: ClickHouse atomicity for merge/split — SUPERSEDED

**Superseded by [ADR-0003](ADR/0003-operator-decisions-as-persons-observations.md)**: merge/split are journal appends in transactional MariaDB (`persons` + `operations`), so the ClickHouse atomicity problem this recommendation worked around no longer exists on the correction path. Retained for history; revisit only if a future matcher needs bulk mutations on ClickHouse-side tables.

### REC-IR-02: Incremental watermark for identity inputs (open)

The persons-seed currently folds the **full** `identity_inputs` set on every run (correct but O(history) — see `cpt-ir-fr-bootstrap-incremental`). Recommended mechanism when it becomes a bottleneck: track a "last processed" position with `_synced_at` as the cursor column and store the high-watermark in a dedicated table keyed by `(insight_tenant_id, job_name)`, updated atomically at the end of each successful run. Note the interaction with the known-account rule: a watermarked run sees only new observations, so binding reuse must keep working from the journal, not from the evidence window.

### REC-IR-03: Shared unmapped table for all domains — RESOLVED

**Decision**: Use a single shared operator queue (owned by the IR domain) for both alias-level and person-attribute-level unresolved observations. See [ADR-0001: Shared unmapped table](../../person/specs/ADR/0001-shared-unmapped-table.md) for full rationale (also listed in §1.2 key decision records). In v1 the shared queue is realised as the derived review queue; the shared persistent table remains the plan if/when the matcher lands.

Reason: identical structure (both carry `insight_tenant_id`, `insight_source_id`, `insight_source_type`, `source_account_id`, `value_type`, `value`) and common data origin (`identity_inputs`). Differentiation by `value_type` values is sufficient — identity value types (`id`, `email`, `username`, `employee_id`, `platform_id`) vs person-attribute types (`display_name`, `role`, `location`, etc.). No separate `person_unmapped` table needed.

### REC-IR-04: Temporary tenant and source ID derivation via sipHash128 (Phase 1)

Phase 1 seed and connector models derive `insight_tenant_id` (UUID) and `insight_source_id` (UUID) from Bronze string `tenant_id` / `source_id` using `toUUID(UUIDNumToString(sipHash128(coalesce(<col>, ''))))`. This is a **temporary** deterministic hash that produces a stable UUID from the raw identifier.

**Formula**: `toUUID(UUIDNumToString(sipHash128(coalesce(<col>, ''))))`
- `sipHash128(...)` — deterministic 128-bit hash (returns `FixedString(16)`)
- `UUIDNumToString(...)` — formats the 16 bytes as a UUID-shaped string
- `toUUID(...)` — parses the string into ClickHouse `UUID` type — **required**: without this outer cast the value is `String` and breaks `UNION ALL` in `identity.identity_inputs` view (error `NO_COMMON_TYPE: UUID, UUID, String, String`). Both the `identity_inputs_from_history` macro (connector models) and all seed models must emit UUID.

**Why temporary**: The PR #55 convention requires `insight_tenant_id` / `insight_source_id` to be real UUIDv7 foreign keys referencing future `tenants` / `sources` tables. Until those exist, the deterministic hash ensures:
- The same Bronze identifier always produces the same UUID across all models.
- No collision risk within realistic tenant counts (sipHash128 is 128-bit).
- `insight_source_id` is query-joinable across `persons`, `account_person_map`, `aliases`, and `identity_inputs` — the journal inherits it from the evidence rows.
- `insight_tenant_id` is **not** cross-store joinable: the evidence carries the producer-side hash while the journal carries the caller's tenant (see §3.2 PersonsSeed and §4.4). Tenant joins across stores become possible only once the `tenants` table replaces the hash.

**Migration path**: When `tenants` / `sources` tables are created, replace all `toUUID(UUIDNumToString(sipHash128(...)))` calls with a lookup join (e.g., `JOIN tenants t ON t.external_id = cm.tenant_id`). All affected files are marked with `-- TEMPORARY: sipHash128` comments. Search: `grep -r "TEMPORARY.*sipHash128" src/ingestion/`.

**Affected files** (Phase 1):
- `dbt/macros/identity_inputs_from_history.sql` — computes both for bamboohr/zoom connector models
- `dbt/identity/seed_persons_from_cursor.sql`, `seed_persons_from_claude_admin.sql` — compute the hash
- `dbt/identity/seed_aliases_from_cursor.sql`, `seed_aliases_from_claude_admin.sql` — use it in tenant-scoped JOINs
- `dbt/identity/seed_identity_inputs_from_cursor.sql`, `seed_identity_inputs_from_claude_admin.sql` — compute the hash
- `scripts/adhoc/seed_from_cursor_manual.sql`, `scripts/adhoc/seed_from_claude_admin_manual.sql` — ad-hoc Play UI testing SQL (point-in-time snapshots, not kept in sync with dbt models)

### REC-IR-05: Explicit canonical id emission per connector (Phase 2)

**Status**: deferred to follow-up PR.

**Context**: the `identity_inputs_from_history` dbt macro currently emits the canonical `value_type='id'` binding observation via two implicit CTEs (`id_upserts`, `id_deletes`) in addition to the per-field `upserts`/`deletes` blocks driven by the connector's `identity_fields` list. Connectors that go through the macro (BambooHR, Zoom, future) get this row automatically; connectors that bypass the macro (`seed_identity_inputs_from_cursor`, `seed_identity_inputs_from_claude_admin`, plus the `scripts/adhoc/seed_from_*_manual.sql` companions) emit it explicitly as a UNION-ALL branch. The contract that "every connector emits a `value_type='id'` observation" is therefore convention-driven, not declarative at the call site.

**Why follow up**: a connector author looking at `zoom__identity_inputss.sql` sees `identity_fields=[email, employee_id, display_name]` and no mention of the account identifier itself. The relationship is invisible without reading the macro. The macro also hardcodes `value_field_name = '{source_type}.entity_id'` for the implicit row instead of the canonical `bronze_<src>.<table>.id` path that explicit entries produce elsewhere.

**Recommended Phase-2 cleanup**:

1. Add `{'field': 'id', 'value_type': 'id', 'value_field_name': 'bronze_<src>.<table>.id'}` explicitly to every connector's `identity_fields` list — Bamboo, Zoom, and any future macro-using connector.
2. Remove `id_upserts` / `id_deletes` from `identity_inputs_from_history`. The per-field `upserts` / `deletes` blocks already handle every declared field uniformly, so the macro becomes simpler and the connector's contract becomes fully explicit.
3. Validate in CI (or in the macro itself) that every connector's `identity_fields` contains exactly one entry with `value_type='id'` — turns the convention into an enforceable contract.

**Why not now**: orthogonal to the schema split, SCD2 cache, and email-conflict policy work in this PR. Bundling would balloon the diff and mix code-style concerns with semantic ones. No bug today — the canonical row IS emitted correctly for every existing connector (verified file-by-file).

**Source**: mitasovr review on commit `bec6c98`, Zoom-thread clarification on `zoom__identity_inputss.sql:15`.

## 6. Traceability

- **PRD**: [PRD.md](./PRD.md)
- **DECOMPOSITION**: [DECOMPOSITION.md](./DECOMPOSITION.md)
- **ADR-0002**: [Stable `person_id` via append-only `persons` observations](ADR/0002-stable-person-id-via-persons-observations.md) — the implemented binding model
- **ADR-0003**: [Operator identity corrections as append-only `persons` observations](ADR/0003-operator-decisions-as-persons-observations.md) — the correction model; supersedes snapshot-based merge/split
- **Reviewed correction design**: constructorfabric/insight#2180 (scenarios S1–S10, API shapes); umbrella vision: #1873
- **Component spec**: `docs/components/backend/identity-resolution/identity/specs/` — the live service (read API, schema rules, service ADRs)
- **Features**: features/ (to be created from DECOMPOSITION entries)
- **Source V2**: `inbox/architecture/IDENTITY_RESOLUTION_V2.md` — MariaDB reference; matching engine, merge/split, API, phases B1–B3
- **Source V3**: `inbox/architecture/IDENTITY_RESOLUTION_V3.md` — Silver layer contract, PostgreSQL added, Bronze → Silver position
- **Source V4 (canonical)**: `inbox/architecture/IDENTITY_RESOLUTION_V4.md` — Golden Record, Source Federation, Conflict Detection, multi-tenancy, SCD2 corrections
- **Source algorithm**: `inbox/IDENTITY_RESOLUTION.md` — ClickHouse-native min-propagation, token/rid model
- **Source walkthrough**: `inbox/architecture/EXAMPLE_IDENTITY_PIPELINE.md` — end-to-end example: Anna Ivanova
- **Related (person domain)**: Person domain DESIGN — golden record, person attributes, person-level conflicts
- **Related (org-chart domain)**: Org-chart domain DESIGN — org_units, person_assignments, SCD Type 2 hierarchy
- **Related (permissions)**: `docs/architecture/permissions/PERMISSION_DESIGN.md` — permission architecture consuming identity data
- **Connectors**: `docs/components/connectors/hr-directory/` — HR connector specifications (sources for Bootstrap)
