> [!WARNING]
> **Under review — audited against the implementation and found inaccurate in places.**
> Several capabilities it calls planned are already shipped. Read it against the code,
> not as authority; the specific claims are listed in the repository
> [README](../../../../README.md#backend-specs--under-review).

# PRD — Identity Resolution

<!-- toc -->

- [1. Overview](#1-overview)
  - [1.1 Purpose](#11-purpose)
  - [1.2 Background / Problem Statement](#12-background--problem-statement)
  - [1.3 Goals (Business Outcomes)](#13-goals-business-outcomes)
  - [1.4 Glossary](#14-glossary)
- [2. Actors](#2-actors)
  - [2.1 Human Actors](#21-human-actors)
  - [2.2 System Actors](#22-system-actors)
- [3. Operational Concept & Environment](#3-operational-concept--environment)
  - [3.1 Module-Specific Environment Constraints](#31-module-specific-environment-constraints)
- [4. Scope](#4-scope)
  - [4.1 In Scope](#41-in-scope)
  - [4.2 Out of Scope](#42-out-of-scope)
- [5. Functional Requirements](#5-functional-requirements)
  - [5.1 Identity Store and Resolution (p1)](#51-identity-store-and-resolution-p1)
  - [5.2 Evidence Intake and Seed Fold (p1)](#52-evidence-intake-and-seed-fold-p1)
  - [5.3 Phase 3 — Matching & Workflows (p2, future)](#53-phase-3--matching--workflows-p2-future)
  - [5.4 Manual Resolution — Operator Corrections (p1)](#54-manual-resolution--operator-corrections-p1)
  - [5.5 Late Phase — GDPR (p3)](#55-late-phase--gdpr-p3)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
  - [6.1 NFR Inclusions](#61-nfr-inclusions)
  - [6.2 NFR Exclusions](#62-nfr-exclusions)
- [7. Public Library Interfaces](#7-public-library-interfaces)
  - [7.1 Public API Surface](#71-public-api-surface)
  - [7.2 External Integration Contracts](#72-external-integration-contracts)
- [8. Use Cases](#8-use-cases)
  - [Bootstrap New Connector Data](#bootstrap-new-connector-data)
  - [Resolve Person for Analytics and Backend](#resolve-person-for-analytics-and-backend)
  - [Review Pending Identity Decisions](#review-pending-identity-decisions)
  - [Merge Two Persons](#merge-two-persons)
  - [GDPR Identity Purge](#gdpr-identity-purge)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Dependencies](#10-dependencies)
- [11. Assumptions](#11-assumptions)
- [12. Risks](#12-risks)

<!-- /toc -->

---

## 1. Overview

### 1.1 Purpose

Identity Resolution maps disparate identity signals — emails, usernames, employee IDs, platform-specific handles — from all connected source systems to canonical person records. It enables cross-system analytics by answering one question: "Which person does this account belong to?" Without it, a Git committer `aivanova`, a Jira assignee `anna.ivanova@acme.com`, and a BambooHR employee `E123` remain three unrelated identities, preventing any meaningful cross-platform productivity or collaboration analysis.

### 1.2 Background / Problem Statement

Insight connects to 10+ external platforms (GitLab, GitHub, Jira, YouTrack, BambooHR, Zoom, M365, Zulip, etc.). Each platform uses its own account model — some identify users by email, others by username, numeric ID, or display name. A single human may appear as 5-15 different identities across these systems.

**Current state**: The original identity resolution monolith handled person records, alias mapping, org hierarchy, golden record assembly, and GDPR deletion in a single design. As part of the domain-split initiative, identity resolution is now scoped to alias-to-person mapping only. Person attributes, org hierarchy, and availability belong to their respective domains.

**Key problems solved**:
- **Fragmented identity**: Commits, issues, messages, and HR records cannot be attributed to the same person without alias resolution
- **Connector diversity**: Each new connector introduces new alias types and naming conventions — the system must handle this uniformly
- **Confidence and safety**: Auto-linking wrong aliases creates corrupt analytics; the system must be conservative and auditable
- **Operational cost**: Without automated bootstrap, operators must manually map every identity — unsustainable at scale

**Target users**: Platform operators managing identity mappings; analytics consumers relying on accurate `person_id` attribution; connectors writing alias observations.

### 1.3 Goals (Business Outcomes)

| Goal | Success Criteria |
|---|---|
| Automated account binding | **Baseline**: 0% auto-bound. **Target**: >= 80% of observed accounts bound automatically within 30 min of connector sync. **Timeframe**: Within 2 sprints of the evidence-intake phase. |
| Zero silent false merges | **Baseline**: N/A (new system). **Target**: 0 automatic merges of two existing persons — structurally excluded; 0 false-positive automatic links over any 90-day window. **Timeframe**: Ongoing. |
| Operator efficiency | **Baseline**: 100% manual mapping. **Target**: Operator reviews only the pending/contested queue — < 20% of observed accounts. **Timeframe**: Within 30 days of the manual-resolution capability shipping. |
| Cross-platform analytics enablement | **Baseline**: Per-platform siloed dashboards. **Target**: 100% of Gold analytics queries use resolved `person_id`. **Timeframe**: Within 1 sprint of the identity-store phase. |
| Audit trail completeness | **Baseline**: No correction tracking. **Target**: 100% of operator corrections are attributable (who/when/why) and reversible. **Timeframe**: From the manual-resolution capability. |

### 1.4 Glossary

| Term | Definition |
|---|---|
| Alias | An `(value_type, value)` pair identifying a person in a specific source system (e.g., `email:anna@acme.com`) |
| Alias type | Category of identity signal: canonical `id`, `email`, `username`, `display_name`; known custom `employee_id`, `platform_id` (deprecated for Cursor/Claude Admin in favor of `id`, see ADR-0002) |
| Bootstrap input | A row in `identity_inputs` representing one changed alias observation from one connector |
| Confidence score | Numeric value (0.0–1.0) representing the MatchingEngine's certainty that an alias belongs to a person |
| Auto-link | (v1 plan vocabulary — superseded by ADR-0003) automatic creation of a link at full confidence. In the implemented model the only automatic link is the seed's unambiguous e-mail bind; the future matcher emits operator-reviewed proposals only |
| Unmapped alias | (v1 plan vocabulary) an alias below the auto-link threshold, queued for review; in the implemented model the equivalent is an account or value pending an operator decision in the derived review queue |
| Alias conflict | When the same `(value_type, value)` is claimed by two different persons |
| Merge | Operator decision that two persons are one human: all account bindings of the absorbed person are reassigned to the surviving person (append-only; see ADR-0003) |
| Split / Detach | Operator decision that an account belongs to a different human: the account is rebound to another (usually freshly minted) person — regardless of how the current grouping arose (see ADR-0003) |
| Hot path | Direct binding lookup against the journal (service read API) or the mirror (analytics) |
| Cold path | Future MatchingEngine rule evaluation when no binding exists |
| Person domain | Separate domain owning the **golden record projection** of persons (and person-level attributes such as availability, conflicts). The MariaDB `persons` identity-attribute history table itself is owned by the identity-resolution domain (see DESIGN §3.7 and ADR-0006); the Person domain reads from it |
| Org-chart domain | Separate domain owning `org_units` and `person_assignments` |
| Reviewer namespace | Code-review identities as reported by platforms — a login/account id with no e-mail; resolved account-first through the id-binding map, with no e-mail bridge |

---

## 2. Actors

> **Note**: Stakeholder needs are managed at project/task level by steering committee. Document **actors** (users, systems) that interact with this module.

### 2.1 Human Actors

#### Operator

**ID**: `cpt-ir-actor-operator`

**Role**: Reviews accounts pending a decision and contested-binding groups, performs corrections (bind, merge, detach, exclude), and — in later phases — manages match-rule configuration and GDPR purge requests. Typically a platform administrator with knowledge of the organization's systems and personnel.

**Needs**: A clear review queue with candidate persons per item; correction verbs covering both directions (same person / different people), individually and in bulk; per-account binding history to understand and audit past decisions; assurance that decisions survive automatic re-runs.

### 2.2 System Actors

#### Connector

**ID**: `cpt-ir-actor-connector`

**Role**: External system connector (e.g., BambooHR, GitLab, Jira) that syncs data from source platforms. Writes alias observations to `identity_inputs` during its sync pipeline, providing raw identity signals for resolution.

#### Automatic Binding Job (persons-seed)

**ID**: `cpt-ir-actor-bootstrap-job`

**Role**: Scheduled job (the `seed` subcommand of the identity-resolution service) that reads observations from `identity_inputs` and folds them into the `persons` journal: reuses existing bindings, links unbound accounts to the person their e-mail unambiguously maps to, mints persons for new e-mails, skips e-mail-less accounts. Never merges two existing persons. Contested-evidence handling (no auto-link when evidence is ambiguous; respect for per-account bindings in divergent groups) ships with the manual-resolution capability. Runs after connector sync cycles.

#### MatchingEngine (future)

**ID**: `cpt-ir-actor-matching-engine`

**Role**: Future rule-evaluation engine. Loads enabled `match_rules`, computes composite confidence scores, and produces merge **proposals** for operator review. Never writes bindings; operator decisions override its output. Not built.

#### Analytics Pipeline

**ID**: `cpt-ir-actor-analytics-pipeline`

**Role**: Downstream consumer (dbt models, dashboards) that resolves `person_id` at build time via the `identity.identity_persons` mirror and the `resolve_person_id` macro. Depends on the journal being accurate; renders unresolved activity with NULL `person_id` rather than guessing.

---

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

- **Storage**: Evidence and the analytics mirror reside in ClickHouse (`identity_inputs`; `identity.identity_persons` published by persons-sync). The identity observation journal `persons`, its derived `org_chart` edges, and the `operations` admin journal reside in MariaDB and are owned by this domain (see DESIGN §3.7, [ADR-0002](ADR/0002-stable-person-id-via-persons-observations.md), [ADR-0003](ADR/0003-operator-decisions-as-persons-observations.md), and ADR-0006). Future matcher tables (`match_rules`, `unmapped`, `conflicts`, `merge_audits`, `alias_gdpr_deleted`) are design proposals only — not built.
- **Orchestration**: the persons-seed runs as a scheduled Kubernetes job (umbrella chart); Argo-orchestrated processing was the earlier plan and is not used by this domain today.
- **Naming**: All tables and columns follow PR #55 glossary conventions (see Glossary and DESIGN §2.2).
- **Temporal model**: Half-open intervals `[effective_from, effective_to)`. `BETWEEN` prohibited on temporal columns. Zero sentinel (`'1970-01-01'`) replaces NULL for ClickHouse compatibility.

---

## 4. Scope

### 4.1 In Scope

- Evidence ingestion: the `identity_inputs` write contract for connectors
- **Persons observation journal**: the MariaDB `persons` table, its scheduled seed from `identity_inputs` (ADR-0002), the `org_chart` edges, and the `operations` journal. Schema / seed / corrections are owned by this domain; the Person domain reads the resulting rows to build its golden record.
- **Operator corrections** (ADR-0003): merge, split/detach, bind (single and bulk), exclude — appended to the journal, surviving re-runs; review queue of accounts pending a decision and contested-binding groups; per-account binding history; per-person account listing
- Analytics resolution path: persons-sync mirror + build-time `person_id` resolution (the dbt resolve macro)
- Person lookups for backend consumers (component spec: `docs/components/backend/identity-resolution/identity/`)
- Matching engine (future): configurable `match_rules`, confidence-scored proposals, normalization pipeline
- GDPR alias deletion (future): hard-erasure flow with archive
- Legacy `aliases` table stewardship until retirement or reuse by the matcher

### 4.2 Out of Scope

- **Person golden record**: assembly of the best-value `persons` record per attribute with source-priority rules, person-level conflict detection — see person domain PRD. (This domain records raw observations; the person domain projects them into a single golden record.)
- **Org hierarchy**: `org_units`, `person_assignments`, SCD Type 2 history — see org-chart domain PRD
- **Connector implementation**: How connectors sync data from external platforms — see connector specifications
- **Permission / RBAC**: Access control, data visibility rules — see permissions domain
- **Metric aggregation**: Gold-layer dashboards, activity summaries — see analytics domain
- **SCD Type 2 for persons/org_units**: Implemented via dbt macros in respective domains

---

## 5. Functional Requirements

> **Testing strategy**: All requirements verified via automated tests (unit, integration, e2e) targeting 90%+ code coverage unless otherwise specified. Document verification method only for non-test approaches (analysis, inspection, demonstration).

### 5.1 Identity Store and Resolution (p1)

> The v1 plan for this phase was alias-table resolution (dbt-seeded `aliases` + a resolve API). The implemented architecture resolves through the `persons` journal instead (DESIGN §1.1); the alias-specific requirements below are marked superseded and retained for traceability.

#### Seed Aliases from HR Bronze Data (superseded)

- [ ] `p1` - **ID**: `cpt-ir-fr-seed-aliases`

> **Superseded.** dbt seed models still populate the legacy `aliases` table, but the resolution path does not consume it (DESIGN §3.7 legacy note). The journal seed requirements (`cpt-ir-fr-persons-history`, `cpt-ir-fr-persons-initial-seed`) replace this capability. Retained for traceability until the table is retired.

#### Resolve Alias to Person (superseded)

- [ ] `p1` - **ID**: `cpt-ir-fr-resolve-alias`

> **Superseded.** Alias-table lookup was never built. Resolution is served by the journal read paths: the identity-resolution service read API for request-time lookups (component spec) and the analytics interface for build-time resolution (`cpt-ir-interface-analytics-resolution`). The insight that account-scoped identifiers are unique only within one source instance survives in the account-first resolver key (DESIGN §4.4).

#### Batch Alias Resolution (superseded)

- [ ] `p1` - **ID**: `cpt-ir-fr-batch-resolve`

> **Superseded.** Bulk resolution happens at build time through the analytics interface (`cpt-ir-interface-analytics-resolution`) — the mirror plus the resolve macro — not through a REST batch endpoint. A request-time batch endpoint may return with the future matcher if a consumer needs it.

#### Tenant Isolation

- [ ] `p1` - **ID**: `cpt-ir-fr-tenant-isolation`

The system **MUST** isolate identity data by `insight_tenant_id`. A resolution request for tenant A **MUST NOT** return identity data belonging to tenant B.

**Known gap**: the seed's evidence read path currently applies no tenant predicate and serves single-tenant deployments only; restoring the tenant filter on `identity_inputs` is a prerequisite for multi-tenant use (see DESIGN §3.2 PersonsSeed).

**Rationale**: Multi-tenant deployments require strict data isolation to prevent cross-tenant data leaks.

**Actors**: `cpt-ir-actor-analytics-pipeline`, `cpt-ir-actor-operator`

#### Persons — Field-Level Identity Attribute History (MariaDB)

- [ ] `p1` - **ID**: `cpt-ir-fr-persons-history`

The system **MUST** maintain a `persons` table in MariaDB that stores the history of identity field changes per person in an append-only SCD-style log. Each row represents one observed field value (`value_type`, `value`) from one source (`insight_source_type`, `insight_source_id`) assigned to a person (`person_id`) at a specific time (`created_at`). Updating a field **MUST** insert a new row rather than mutate an existing one; the current value is the row with the latest `created_at` for the `(insight_tenant_id, person_id, value_type)` triple.

Schema (the column split by `value_type` into `value_id` / `value_full_text` / `value`, the generated display/digest columns, the natural-key UNIQUE ending in `created_at`, and microsecond timestamp precision) is specified in the identity-resolution DESIGN §3.7 and in [ADR-0002](ADR/0002-stable-person-id-via-persons-observations.md) with its post-acceptance note. The PRD intentionally states only the behavioural contract.

**Rationale**: Downstream backend services (Analytics API, Identity Resolution service) need a CRUD-accessible, temporal view of person attributes from heterogeneous sources — ClickHouse is optimised for analytical reads, not operator-driven edits. MariaDB with row-level history supports conflict-resolution UX, audit trails, and operator-driven corrections.

**Actors**: `cpt-ir-actor-analytics-pipeline`, `cpt-ir-actor-operator`

#### Initial Seed of Persons from identity_inputs

- [ ] `p1` - **ID**: `cpt-ir-fr-persons-initial-seed`

The system **MUST** provide a re-runnable bootstrap mechanism that populates `persons` from `identity_inputs` with the following behavioural guarantees:

- **Email as identity anchor**: each source-account's email observation uniquely identifies a person within a tenant; source-accounts without an email **MUST** be skipped.
- **Stable identity assignment**: within an initial-bootstrap run, source-accounts sharing a normalised email within a tenant are assigned one randomly-minted `person_id` (UUIDv7); once assigned, the binding is never re-derived from mutable attributes and survives re-runs. See [ADR-0002](ADR/0002-stable-person-id-via-persons-observations.md).
- **Idempotent re-run**: running the bootstrap on identical input **MUST NOT** add any rows; running after a new connector sync **MUST** add only the newly observed rows without losing or duplicating prior ones.
- **Non-destructive**: the bootstrap **MUST NOT** delete, truncate, or otherwise remove rows from `persons`. Re-seeding from scratch is an explicit operator action outside the bootstrap.

**Rationale**: Bootstraps the `persons` table with every person who has an email anchor across configured connectors. Email is the single authoritative cross-source identity key in Phase 1 (matching rules come in Phase 3). The deterministic-identity + idempotent-re-run guarantees together mean the bootstrap is safe to execute after every connector sync without duplicating data or wiping operator-authored rows.

Schema of `persons`, UNIQUE-key structure, `person_id` minting (random UUIDv7), and script layout are specified in the identity-resolution DESIGN §3.7 "Table: `persons`" and in [ADR-0002](ADR/0002-stable-person-id-via-persons-observations.md).

**Actors**: `cpt-ir-actor-analytics-pipeline`

### 5.2 Evidence Intake and Seed Fold (p1)

> The v1 plan for this phase was a BootstrapJob maintaining the `aliases` store with `unmapped`/`conflicts` queues. What shipped is the persons-seed fold into the journal (DESIGN §3.2); superseded alias-pipeline requirements are marked below and retained for traceability.

#### Accept Alias Observations from Connectors

- [x] `p1` - **ID**: `cpt-ir-fr-accept-bootstrap-inputs`

The system **MUST** accept alias observation records into the `identity_inputs` table. Each record **MUST** include: `insight_tenant_id`, `insight_source_id`, `insight_source_type`, `source_account_id`, `value_type`, `value`, `value_field_name`, `operation_type` (UPSERT or DELETE).

**Rationale**: Connectors need a uniform write target for identity signals. The `identity_inputs` table decouples connector sync from alias resolution timing.

**Actors**: `cpt-ir-actor-connector`

#### Process Bootstrap Inputs Incrementally

- [ ] `p1` - **ID**: `cpt-ir-fr-bootstrap-incremental`

The seed **MUST** process `identity_inputs` such that re-runs neither duplicate nor lose observations, and **SHOULD** process incrementally — reading only rows newer than a persisted watermark and advancing it after each successful run.

**Current state**: not implemented as incremental — each run folds the **full** evidence set (idempotent via the journal's natural key); the watermark mechanism is an open scalability requirement (DESIGN §5 REC-IR-02).

**Rationale**: Connectors sync continuously; full-set folding is correct but its cost grows with history.

**Actors**: `cpt-ir-actor-bootstrap-job`

#### Normalize Alias Values (superseded)

- [ ] `p1` - **ID**: `cpt-ir-fr-normalize-aliases`

> **Superseded.** Normalization now lives at the consumption points: the seed groups accounts by lowercased e-mail; the analytics macro applies `lower(trim(...))` to **both** join sides; `value_id` comparisons in the journal are case-insensitive by collation (service migration 004). Raw values in `identity_inputs` remain preserved unchanged — that part of the contract stands via `cpt-ir-contract-bootstrap-inputs`.

#### Create Alias on Exact Match (superseded)

- [ ] `p1` - **ID**: `cpt-ir-fr-create-alias-exact`

> **Superseded.** Automatic exact-match linking is implemented by the seed's unambiguous e-mail link (an unbound account whose e-mail maps to exactly one person joins that person — DESIGN §3.2 PersonsSeed). Anything beyond that arrives with the future MatchingEngine strictly as **operator-reviewed proposals** — the matcher never writes bindings (ADR-0003).

#### Route Low-Confidence Aliases to Unmapped Queue (future — with the matcher)

- [ ] `p3` - **ID**: `cpt-ir-fr-route-unmapped`

When the future MatchingEngine produces a candidate at any confidence, the system **MUST** surface it for operator review as a proposal (with the candidate person and its confidence) rather than silently dropping or silently applying it; confidence bands only order proposals and gate bulk acceptance. Whether suggestions persist in a store or derive on read is a design decision for that iteration; in v1 the review need is covered by the derived queue (`cpt-ir-fr-review-queue`).

**Rationale**: Matcher candidates must not be silently dropped — and per ADR-0003 must not be silently applied either.

**Actors**: `cpt-ir-actor-bootstrap-job`, `cpt-ir-actor-matching-engine`

#### Track Alias Observations Over Time (superseded)

- [ ] `p1` - **ID**: `cpt-ir-fr-track-observations`

> **Superseded.** The append-only journal records every observation with its own `created_at` (sourced from `_synced_at`), so observation history is first-class rather than a pair of `first/last_observed_at` columns on a mutable alias row.

#### Idempotent Bootstrap Runs (superseded)

- [ ] `p1` - **ID**: `cpt-ir-fr-bootstrap-idempotent`

> **Superseded.** Idempotent re-runs are guaranteed at the journal level: `INSERT IGNORE` under the natural observation key ending in `created_at`, with `created_at` sourced deterministically from `_synced_at` (see `cpt-ir-fr-persons-initial-seed` and `cpt-ir-nfr-bootstrap-idempotency`).

### 5.3 Phase 3 — Matching & Workflows (p2, future)

> Nothing in this phase is implemented; these requirements govern the future MatchingEngine (DESIGN §4.2). Operator-workflow requirements that ADR-0003 re-scoped are marked superseded in favour of §5.4.

#### Configurable Match Rules

- [ ] `p2` - **ID**: `cpt-ir-fr-configurable-rules`

The system **MUST** allow operators to view, enable/disable, and adjust weights of match rules via the API. Each rule **MUST** have a `rule_type`, `phase`, `condition_type`, `weight`, `is_enabled`, and `sort_order`.

**Rationale**: Different deployments have different identity landscapes. Operators need to tune matching rules for their specific source mix without code changes.

**Actors**: `cpt-ir-actor-operator`

#### Three-Phase Matching Pipeline

- [ ] `p2` - **ID**: `cpt-ir-fr-three-phase-matching`

The MatchingEngine **MUST** evaluate rules in three ordered phases: B1 (deterministic — exact email, exact HR ID), B2 (normalization and cross-system — case-insensitive email, domain aliases, cross-system username), B3 (fuzzy — Jaro-Winkler, Soundex). The system **MUST** compute a composite confidence score from weighted rule matches.

**Rationale**: Phased matching provides escalating specificity. Deterministic rules run first for speed; fuzzy rules run last and only when deterministic matching fails.

**Actors**: `cpt-ir-actor-matching-engine`

#### No Fuzzy Auto-Link

- [ ] `p2` - **ID**: `cpt-ir-fr-no-fuzzy-autolink`

Fuzzy matching rules (Phase B3) **MUST NEVER** trigger automatic binding creation regardless of confidence score. Their output **MUST** be operator-reviewed proposals only and **MUST NOT** be eligible for bulk acceptance. (Under ADR-0003 no matcher rule of any phase writes bindings; this requirement additionally hardens the fuzzy tier.)

**Rationale**: Fuzzy name matching is a known source of false-positive merges, which corrupt attribution and are costly to unwind. This constraint is non-negotiable.

**Actors**: `cpt-ir-actor-matching-engine`

#### Operator Unmapped Queue Management (superseded)

- [ ] `p2` - **ID**: `cpt-ir-fr-unmapped-management`

> **Superseded** by the §5.4 operator-correction requirements (`cpt-ir-fr-review-queue`, `cpt-ir-fr-operator-bind`, `cpt-ir-fr-operator-exclude`): the queue is derived rather than a stateful `unmapped` record workflow. Retained for traceability; a persistent proposal store may return with the matcher (see `cpt-ir-fr-route-unmapped`).

#### Alias Conflict Detection (superseded)

- [ ] `p2` - **ID**: `cpt-ir-fr-alias-conflict-detection`

> **Superseded.** Conflict surfacing is covered by the derived review queue (`cpt-ir-fr-review-queue`): contested identity values and unexplained binding divergence are surfaced with candidates, without a persistent `conflicts` table. A persistent conflict record may return with the matcher.

#### Manual Alias Management (superseded)

- [ ] `p2` - **ID**: `cpt-ir-fr-manual-alias-crud`

> **Superseded** by the §5.4 operator corrections: attaching identity to a person is `cpt-ir-fr-operator-bind`; taking an account away is `cpt-ir-fr-split-v2` / `cpt-ir-fr-operator-exclude`; listing a person's identity values is `cpt-ir-fr-binding-history`; auditability is `cpt-ir-fr-merge-audit-v2` (journal, not `merge_audits`).

#### Auto-Resolve Unmapped on New Alias (future — with the matcher)

- [ ] `p3` - **ID**: `cpt-ir-fr-auto-resolve-unmapped`

If the matcher iteration introduces a persistent suggestion store, newly established bindings **SHOULD** auto-resolve pending suggestions for the same identity value. In v1 this is inherent: the review queue is derived, so an item disappears as soon as new bindings dissolve its condition.

**Actors**: `cpt-ir-actor-bootstrap-job`

### 5.4 Manual Resolution — Operator Corrections (p1)

> Requirements for the operator correction capability decided in [ADR-0003](ADR/0003-operator-decisions-as-persons-observations.md) (reviewed design: constructorfabric/insight#2180). The v1 late-phase merge/split/audit/idempotency requirements prescribed a snapshot-and-rollback mechanism that was never built; they are superseded by the implementation-neutral `-v2` requirements below (see this file's git history for the v1 texts).

#### Merge Persons

- [ ] `p1` - **ID**: `cpt-ir-fr-merge-v2`

The system **MUST** allow an operator to declare that two persons are the same human. All account bindings of the absorbed person **MUST** be reassigned to the surviving person, which the operator names explicitly. The operation **MUST** be recorded as new history (no existing records modified or deleted), attributed to the operator with a reason.

**Rationale**: Automatic resolution under-merges (e.g., work vs personal e-mail). Operators need to combine persons without losing any history. (Supersedes the v1 merge requirement.)

**Actors**: `cpt-ir-actor-operator`

#### Split / Detach Accounts

- [ ] `p1` - **ID**: `cpt-ir-fr-split-v2`

The system **MUST** allow an operator to declare that an account belongs to a different human: rebind the account to another person — usually a newly created one — **regardless of how the current grouping arose**. No prior merge record may be required. The operation **MUST** be recorded as new history, attributed to the operator with a reason.

**Rationale**: Automatic resolution can over-merge (e.g., through a shared mailbox such as `team@example.com`). Splitting must therefore work on accounts the automation grouped, where no earlier "merge" exists to roll back. (Supersedes the v1 split requirement.)

**Actors**: `cpt-ir-actor-operator`

#### Bind Account to Person (single, bulk, pre-registration)

- [ ] `p1` - **ID**: `cpt-ir-fr-operator-bind`

The system **MUST** allow an operator to attach an account to a chosen person: individually, or in bulk from a prepared matching table. Bulk rows **MAY** address an account directly or via an observed value (e-mail, username); a value that does not resolve to exactly one account **MUST** be reported per-row and left undecided, never guessed. Binding an account that has not been observed yet **MUST** be accepted and take effect when the account first appears. Binding an account pending review to its own current person **MUST** count as an operator confirmation and clear it from review.

**Rationale**: Covers linking accounts pending review, importing a prepared e-mail/username matching table (for example, one exported from an HR system or maintained as a spreadsheet), and pre-registering known accounts of new hires.

**Actors**: `cpt-ir-actor-operator`

#### Exclude Non-Person Accounts

- [ ] `p1` - **ID**: `cpt-ir-fr-operator-exclude`

The system **MUST** allow an operator to mark an account as not belonging to any human (bot, CI, service account). Activity of excluded accounts **MUST NOT** be attributed to any person, and excluded accounts **MUST NOT** appear in the review queue.

**Rationale**: Bot and service accounts (e.g., `ci-bot@example.com`) are not humans; they must be excludable, not merged into persons or endlessly re-reviewed.

**Actors**: `cpt-ir-actor-operator`

#### Review Queue of Pending Decisions

- [ ] `p1` - **ID**: `cpt-ir-fr-review-queue`

The system **MUST** surface the accounts that require an operator decision: (a) accounts whose identity evidence is contested (e.g. an e-mail claimed by more than one person), each with candidate persons and the observed values that make it contested; (b) binding divergence within an identity-value group that is **not** explained by any operator-authored decision; (c) observed accounts with no usable identity evidence (e.g. e-mail-less) — visible and countable, never hidden. Divergence explained by an operator decision **MUST NOT** be surfaced. A surfaced item **MUST** disappear once a decision removes its condition, without any separate item lifecycle to maintain. Alongside the queue, the system **MUST** report resolution-rate shares — how many observed accounts are bound, pending, without evidence, and excluded — so the match rate is operator-visible. Deliberate MVP narrowing: there is no ignore/defer (snooze) action — an item leaves the queue only through a decision (bind, detach, exclude, or confirm); deferral state returns with the proposal store of the matcher iteration.

**Rationale**: Operators need one place showing what needs attention, and it must not show what a human already settled; unresolved is a first-class surface, and reported match rate is a success measure of the umbrella epic (#1873). Surfacing e-mail-less accounts closes #1776. (Together with `cpt-ir-fr-operator-bind` and `cpt-ir-fr-operator-exclude`, supersedes the queue workflow of `cpt-ir-fr-unmapped-management`.)

**Actors**: `cpt-ir-actor-operator`

#### Corrections Survive Automatic Re-Resolution

- [ ] `p1` - **ID**: `cpt-ir-fr-correction-durability`

An operator decision **MUST** survive every subsequent run of automatic resolution and the connection of any new source: automation **MUST NOT** override, re-derive, or silently supersede an operator-authored binding under any circumstances. (Unconditional once the manual-resolution seed hardening is deployed; until then the divergent-group gap documented in DESIGN §3.2 remains the one known violation path.)

**Rationale**: The single most repeated stakeholder requirement (#1873): the override store is the source of truth; automation only proposes.

**Actors**: `cpt-ir-actor-operator`

#### Correction Audit and Reconstructibility

- [ ] `p1` - **ID**: `cpt-ir-fr-merge-audit-v2`

Every correction **MUST** be recorded with the operator identity, a machine-readable reason, an optional free-text comment, and a timestamp — and **MUST NOT** modify or delete any prior record. The retained records **MUST** be sufficient to reconstruct the state before any correction and to answer "who decided this, when, and why" for any current binding.

**Rationale**: Auditability and reversibility without snapshot machinery: history that is never destroyed is its own audit trail. (Supersedes the v1 audit-trail requirement.)

**Actors**: `cpt-ir-actor-operator`

#### Idempotent Corrections

- [ ] `p1` - **ID**: `cpt-ir-fr-idempotent-mutations-v2`

Re-applying a correction identical to an **already-recorded operator decision** (operator retry, duplicate bulk row, re-uploaded matching table) **MUST NOT** change resolution state or duplicate records. Confirming an automation-made binding is not a repeat: it **MUST** record the operator's decision (see the bind requirement).

**Rationale**: Retries and re-uploads must be safe by construction, without client-side idempotency plumbing. (Supersedes the v1 idempotency requirement.)

**Actors**: `cpt-ir-actor-operator`

#### Binding History per Account and per Person

- [ ] `p2` - **ID**: `cpt-ir-fr-binding-history`

The system **MUST** let an operator view, for any account, its current binding and the full decision history (author, reason, time of each change); and, for any person, every account and identity value ever bound to them with the author of each link.

**Rationale**: Operators cannot make safe merge/split decisions without seeing why the current state is what it is; the same view supports troubleshooting attribution questions.

**Actors**: `cpt-ir-actor-operator`

### 5.5 Late Phase — GDPR (p3)

#### GDPR Alias Purge

- [ ] `p3` - **ID**: `cpt-ir-fr-gdpr-purge`

The system **MUST** support GDPR hard erasure of a person's identity data, after which the erased values **MUST NOT** be resolvable via any path and no plaintext copy of them may be retained anywhere, including archives. Erasure **MUST** be an explicit administrative operation, recorded in the operations journal; the append-only rule of the decision journal governs identity decisions and does not preclude lawful erasure of stored values.

**Rationale**: Legal compliance with right-to-erasure requests. Identity data contains PII (emails, names, employee IDs).

**Actors**: `cpt-ir-actor-operator`

---

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

#### Alias Lookup Latency

- [ ] `p1` - **ID**: `cpt-ir-nfr-alias-lookup-latency`

The system **MUST** resolve a single person lookup via the service read API in < 50 ms at p99 under normal load.

**Threshold**: p99 latency < 50 ms for a profile lookup that hits an existing binding, measured at 1000 req/s sustained.

**Rationale**: Resolution is on the critical path for Silver step 2 enrichment. High latency blocks analytical pipeline throughput.

#### Bootstrap Throughput

- [ ] `p1` - **ID**: `cpt-ir-nfr-bootstrap-throughput`

The seed **MUST** process at least 100,000 `identity_inputs` rows per run within 30 minutes.

**Threshold**: >= 100K rows processed in <= 30 min on standard cluster resources (0.5 CPU, 512 MB RAM).

**Rationale**: Large connector syncs (BambooHR with 50K employees, GitLab with 100K users) must complete within the dashboard visibility SLA.

#### Bootstrap Idempotency

- [ ] `p1` - **ID**: `cpt-ir-nfr-bootstrap-idempotency`

Re-running the seed on identical input **MUST** produce identical output — zero net new journal rows, zero net deleted rows.

**Threshold**: After 3 consecutive runs on unchanged data, `SELECT count() FROM persons` returns the same value and `org_chart` rebuilds bit-identically.

**Rationale**: System restarts, Argo retries, and operational re-runs must be safe.

#### No Fuzzy Auto-Link Safety

- [ ] `p2` - **ID**: `cpt-ir-nfr-no-fuzzy-autolink`

The system **MUST** produce zero false-positive auto-links from fuzzy matching rules over any 90-day production window.

**Threshold**: 0 auto-created aliases traced to Phase B3 fuzzy rules.

**Rationale**: False merges corrupt analytics and are extremely costly to unwind. This is a hard safety constraint.

#### Tenant Data Isolation

- [ ] `p1` - **ID**: `cpt-ir-nfr-tenant-isolation`

A resolution request for tenant A **MUST NOT** return data from tenant B under any circumstances, including cache hits, mirror reads, and error responses.

**Threshold**: 0 cross-tenant data leaks in penetration testing.

**Rationale**: Multi-tenant SaaS compliance requirement.

#### GDPR Erasure Completeness

- [ ] `p3` - **ID**: `cpt-ir-nfr-gdpr-erasure`

After a GDPR purge, the purged identity values **MUST NOT** be resolvable via any path (service API, analytics mirror, direct table query) within 60 minutes.

**Threshold**: A purged value resolves to no person via every read path within 60 min of purge, including the next mirror publish.

**Rationale**: Legal compliance with right-to-erasure. Delayed purge visibility is a regulatory risk.

#### Correction Reversibility

- [ ] `p1` - **ID**: `cpt-ir-nfr-merge-reversibility`

Every operator correction **MUST** be reversible: applying a correction and then the counter-correction **MUST** restore the effective account-to-person state, and the state before any correction **MUST** remain reconstructible from retained history at all times.

**Threshold**: 100% round-trip fidelity — effective bindings after correction + counter-correction are identical to the pre-correction bindings; reconstruction of the pre-correction state succeeds for every corrected account.

**Rationale**: Operator confidence and data integrity. Irreversible merges are the single most documented failure mode of comparable systems; reversibility must never depend on remembering to snapshot.

### 6.2 NFR Exclusions

- **High availability / clustering**: Identity resolution is not on the real-time serving path for end users. ClickHouse cluster availability is managed at infrastructure level, not by this domain.
- **Sub-second consistency**: The analytical pipeline resolves at build cadence (a correction becomes visible in dashboards on the next gold build). Real-time consistency is not required.
- **Encryption at rest**: Handled by ClickHouse infrastructure configuration, not by application-level encryption in this domain.

---

## 7. Public Library Interfaces

### 7.1 Public API Surface

#### Identity Resolution REST API

- [ ] `p1` - **ID**: `cpt-ir-interface-resolution-api`

**Type**: REST API (HTTP/JSON), served by the `identity-resolution` service

**Stability**: read surface stable; operator correction surface planned (contracts fixed at FEATURE level; design reviewed in constructorfabric/insight#2180)

**Description**: Person lookups for backend consumers (see the component spec) plus the operator correction surface: bind (single/bulk), merge, detach, exclude, review queue, binding history. Match-rule configuration and GDPR purge join this interface with their future phases.

**Breaking Change Policy**: Endpoint paths and response shapes are versioned; breaking changes require a major version bump.

#### Analytics Resolution Interface (mirror + macro)

- [ ] `p1` - **ID**: `cpt-ir-interface-analytics-resolution`

**Type**: ClickHouse mirror table (`identity.identity_persons`) + dbt macro (`resolve_person_id`)

**Stability**: stable

**Description**: The analytical read path. persons-sync republishes the journal into ClickHouse atomically; every gold model resolves `person_id` at build time through the macro — the single place resolution semantics live for analytics. Unresolved activity carries NULL `person_id` (absent, never guessed).

**Breaking Change Policy**: Macro semantics changes propagate to all consuming models on the next build; they require a coordinated dbt release note. (Replaces the v1 Dictionary-over-`aliases` interface, retired with the legacy table.)

### 7.2 External Integration Contracts

#### Bootstrap Inputs Write Contract

- [x] `p1` - **ID**: `cpt-ir-contract-bootstrap-inputs`

**Direction**: required from client (connectors)

**Protocol/Format**: ClickHouse INSERT (native protocol or HTTP interface)

**Description**: Connectors **MUST** write alias observations to the `identity_inputs` table with all required fields (`insight_tenant_id`, `insight_source_id`, `insight_source_type`, `source_account_id`, `value_type`, `value`, `value_field_name`, `operation_type`). The `value_field_name` **MUST** be fully-qualified: `bronze_{descriptor.name}.{table}.{field}[.json_path]`.

**Compatibility**: Additive columns are backward-compatible. Removing or renaming required columns is a breaking change.

#### Person Domain Cross-Reference Contract

- [ ] `p1` - **ID**: `cpt-ir-contract-person-domain`

**Direction**: provided by this domain (identity resolution provides `persons` observations and mints `person_id`)

**Protocol/Format**: read access to the `persons` observation journal; `person_id` is the stable random UUIDv7 minted at first binding (ADR-0002), never the auto-increment observation row PK

**Description**: The `persons` journal is the authoritative source for the account-to-person binding and for identity-attribute observations. This domain mints `person_id`; the person domain reads the observations to derive its golden record and never writes here. (The v1 wording of this contract — `aliases.person_id` as the authoritative mapping, persons created by the person domain — is superseded; the legacy `aliases` table is not consumed by resolution.)

**Compatibility**: The `person_id` UUID format is stable. Journal column semantics changes are breaking for the person domain.

---

## 8. Use Cases

### Bootstrap New Connector Data

- [ ] `p1` - **ID**: `cpt-ir-usecase-bootstrap`

**Actor**: `cpt-ir-actor-bootstrap-job`, `cpt-ir-actor-connector`

**Preconditions**:
- Connector has completed a sync; dbt models have written its observations to `identity_inputs`
- The seed run is triggered (scheduled) or invoked manually

**Main Flow**:
1. The seed reads `identity_inputs` observations and groups them per source account
2. Accounts with an existing binding are left bound; their new observations are appended under the bound person
3. Unknown accounts with a new e-mail get a freshly minted person (accounts sharing the same new e-mail are grouped)
4. Unbound accounts whose e-mail unambiguously maps to one person are linked to it; contested evidence (an e-mail claimed by more than one person) **MUST NOT** be auto-linked — it is surfaced for review (target behaviour of the manual-resolution capability)
5. Binding divergence within an e-mail group is classified by author; only seed-authored divergence is surfaced
6. The derived cache is rebuilt; the run and its counters are journaled

**Postconditions**:
- Every processed account is bound, surfaced for review, or (e-mail-less) skipped
- Operator decisions from before the run are untouched
- Run summary (including conflict counters) is available in the operations journal

**Alternative Flows**:
- **Seed fails or is refused by an input guard**: no partial bindings are visible; safe to retry (idempotent); guard overrides require an explicit operator flag
- **Concurrent run**: the run-lock makes the second invocation exit without effect

---

### Resolve Person for Analytics and Backend

- [ ] `p1` - **ID**: `cpt-ir-usecase-resolve-hot`

**Actor**: `cpt-ir-actor-analytics-pipeline`

**Preconditions**:
- The journal mirror (`identity.identity_persons`) has been published

**Main Flow** (analytics, bulk):
1. A gold build calls the `resolve_person_id` macro over the mirror
2. Each fact keyed by a source account receives the `person_id` of the account's latest binding
3. Unresolved accounts yield NULL; excluded accounts resolve to NULL

**Postconditions**:
- All gold tables of the build agree on person attribution (build-scoped snapshot)

**Alternative Flows**:
- **Backend request-time lookup**: a service calls the identity read API (`/v1/profiles`); an ambiguous lookup (single-result invariant violated) returns an explicit ambiguity error rather than picking a winner

---

### Review Pending Identity Decisions

- [ ] `p1` - **ID**: `cpt-ir-usecase-review-unmapped`

**Actor**: `cpt-ir-actor-operator`

**Preconditions**:
- Accounts pending a decision and/or unexplained binding conflicts exist
- Operator has the identity correction role

**Main Flow**:
1. Operator requests the review queue and sees each pending account with its observed values and candidate persons
2. Operator investigates using per-account binding history when needed
3. Operator decides: bind to a candidate, detach as a separate person, confirm the current person (bind-to-self), or exclude as a non-person
4. The decision is appended to the journal under the operator's identity; the queue item disappears because its condition no longer holds

**Postconditions**:
- The account's effective binding reflects the operator decision
- The decision survives all future automatic runs

**Alternative Flows**:
- **Bulk import**: the operator uploads a prepared matching table via bulk bind; unambiguous rows apply, ambiguous rows are reported per-row and stay in the queue
- **Legacy conflict**: for a seed-authored binding conflict, the operator either merges the persons or re-asserts the divergence under their own authorship, which silences it

---

### Merge Two Persons

- [ ] `p1` - **ID**: `cpt-ir-usecase-merge`

**Actor**: `cpt-ir-actor-operator`

**Preconditions**:
- Two persons exist that the operator has determined represent the same human

**Main Flow**:
1. Operator requests a merge, naming the surviving person explicitly, with a reason
2. The system appends a binding to the survivor for every account of the absorbed person, authored by the operator
3. The operation (actor, request, comment) is journaled
4. On the next build, all historical activity of the absorbed person's accounts re-attributes to the survivor

**Postconditions**:
- The absorbed person has no current accounts (its history remains intact)
- The merge is attributable (who, when, why) and reversible via detach

**Alternative Flows**:
- **Merge was wrong**: the operator detaches the affected accounts to a new person (counter-action); no rollback machinery is involved
- **Same-person merge**: merging a person into itself is rejected as a no-op validation error

---

### GDPR Identity Purge

- [ ] `p3` - **ID**: `cpt-ir-usecase-gdpr-purge`

**Actor**: `cpt-ir-actor-operator`

**Preconditions**:
- A right-to-erasure request has been received for a specific person
- The purge flow (late phase) is implemented

**Main Flow**:
1. Operator invokes the purge for the person; the operation (actor, subject, time, reason) is recorded in the operations journal
2. The system erases the person's stored identity values everywhere they physically rest: the value payloads of the person's `persons` observations, the rows of the per-connector staging tables behind the `identity_inputs` union view (the view itself stores nothing), and the legacy `aliases` rows — structural rows may survive only value-free
3. Upstream copies the evidence derives from (connector history/raw layers) are erased through the owning ingestion domain's purge hook — a stated prerequisite of this flow: without it, a pipeline rebuild could re-materialize the values
4. The next journal mirror publish and gold build no longer contain the erased values; derived caches rebuild without them; transformations consult the deny-list (step 5) so an erased value cannot re-materialize even on rebuild
5. The purge records value-free tombstones in a deny-list consulted by automation and transformations: a keyed digest (HMAC-SHA256 of the normalized value) under a **versioned per-tenant key** held in the platform secret store. Keys are versioned, not rotated-in-place: each tombstone records the key version it was written under; new tombstones use the current version; a candidate value is checked by computing its digest under every retained key version — erased plaintext cannot be re-digested, so key versions with live tombstones **MUST** be retained for the deny-list to function (they are secrets: key plus digest permits verifying guessed values, so versions live in the secret store under the same access control as the current key). Destroying a key version is a separate, explicitly-confirmed administrative act that crypto-shreds the tombstones written under it and **forfeits their re-link protection entirely**: the system can no longer recognise a re-delivered copy of those values, and it will flow through ordinary automatic processing (it may auto-link or mint a person like any new evidence). The destruction record in the operations journal **MUST** state this consequence

**Postconditions**:
- Erased values are unresolvable via any path (service API, analytics mirror, direct query)
- No plaintext copy of an erased value is retained anywhere, including archives

**Alternative Flows**:
- **No values found for the person**: purge succeeds with a zero count
- **A connector re-delivers an erased value**: the deny-list digest matches; automatic linking is blocked and the account surfaces for operator review
- **The ingestion-domain purge hook is unavailable**: the purge reports partial completion and the erasure request stays open — domain-local erasure alone does not satisfy the requirement

> The v2.0 mechanism — copying purged aliases into a plaintext `alias_gdpr_deleted` archive with a soft-delete in `aliases` — is rejected: retaining the values contradicts hard erasure, and it never touched `persons`, where the values actually rest (see DESIGN §3.7).

---

## 9. Acceptance Criteria

- [ ] Accounts observed by connectors are bound by the seed or surfaced for review by the evidence-derived queue (e-mail-less accounts included); bound accounts resolve to a `person_id` in gold builds and via the read API
- [ ] The seed processes 100K `identity_inputs` rows without duplicates; three consecutive runs on unchanged data change nothing
- [ ] An operator-authored binding survives a subsequent seed run and the connection of a new source, byte-for-byte
- [ ] Accounts pending a decision appear in the review queue with their candidate persons; a decision removes the item without any explicit "close" step
- [ ] Observed accounts with no identity evidence appear in the review queue (countable), never hidden; the queue reports resolution-rate shares
- [ ] Binding divergence explained by an operator decision is not surfaced as a conflict; unexplained seed-authored divergence is
- [ ] Merge, detach, bind (single and bulk), and exclude each append history only — no row of `persons` is ever updated or deleted
- [ ] A correction followed by its counter-correction restores the effective bindings (reversibility)
- [ ] Re-submitting an identical correction or bulk file is a no-op
- [ ] After a correction, the next gold build re-attributes the affected accounts' complete history, including past periods
- [ ] Cross-tenant resolution returns empty for mismatched `insight_tenant_id`
- [ ] (Future, with the matcher) No fuzzy rule produces an auto-applied merge under any input
- [ ] (Future, with the purge flow) GDPR purge renders identity values unresolvable within 60 minutes

---

## 10. Dependencies

| Dependency | Description | Criticality |
|---|---|---|
| ClickHouse 24.x+ | Evidence store (`identity_inputs`) and journal mirror; `generateUUIDv7()` support required | `p1` |
| MariaDB | Journal store (`persons`, `org_chart`, `operations`); schema applied by the identity-resolution service (ADR-0006) | `p1` |
| dbt models (Bronze → Silver) | Populate `identity_inputs` during connector sync transformations; resolve `person_id` at build time via the macro | `p1` |
| Kubernetes scheduling (umbrella chart) | Runs the persons-seed on schedule and hosts the identity-resolution service | `p1` |
| Connector sync pipeline | Writes identity observations to `identity_inputs`; must conform to the write contract | `p1` |
| Person domain | Reads `persons` observations to build the golden record; consumer, not a prerequisite — this domain mints `person_id` (ADR-0002) | `p2` |

---

## 11. Assumptions

- `person_id` is minted by this domain (random UUIDv7 at first binding, ADR-0002); the person domain derives its golden record from this domain's observations, not the other way around.
- Connectors conform to the `identity_inputs` write contract and provide accurate `value_field_name` values.
- ClickHouse 24.x+ is available in all deployment environments with `generateUUIDv7()` support.
- The canonical value types (`id`, `email`, `username`, `display_name`) plus known custom types cover current connector identity signals. New types can be added without schema changes.
- E-mail is the sole automatic identity anchor in the current phase; richer signals arrive with the future matcher.
- A single operator per tenant performs corrections in the current phase; concurrent multi-operator workflows are a revisit trigger of ADR-0003.
- The operator reviews the queue regularly; backlog growth is monitored via queue counts and the coverage view.

---

## 12. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| Connector writes malformed `identity_inputs` rows | Seed fails or creates incorrect bindings | Write contract validation at ingestion; malformed rows logged and skipped |
| Conservative automation (review-first for contested evidence) | Queue grows after each new connector; operator burden increases | Bulk bind for prepared matching tables; queue counts monitored; future matcher proposals reduce per-item work |
| Operator error in corrections | Wrong merge/detach mis-attributes metrics until noticed | History is never destroyed: counter-action restores state; per-account history supports investigation; single-operator assumption limits contention |
| E-mail-less accounts cannot be auto-bound | Their activity stays unattributed until an operator binds them | Never hidden: surfaced in the review queue from evidence; operator `bind` covers them (#1776) |
| Scale: large organization with > 100K persons | Seed throughput or lookup latency degrades | Benchmark at 100K+ scale; indexes sized in DESIGN §3.7 |
| Future matcher reintroduces silent merges | Corrupted attribution, loss of operator trust | ADR-0003 invariants: proposals only, journal decisions override rules; no-fuzzy-autolink NFR |
| Domain boundary misunderstanding | Teams accidentally put person-domain logic in identity-resolution | Clear scope documentation (§4); code review enforcement of domain boundaries |
