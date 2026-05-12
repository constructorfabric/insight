# Decomposition: Identity Resolution

<!-- toc -->

- [1. Overview](#1-overview)
- [2. Entries](#2-entries)
  - [2.1 Initial Seed — HIGH](#21-initial-seed--high)
  - [2.2 Bootstrap Pipeline — HIGH](#22-bootstrap-pipeline--high)
  - [2.3 Matching Engine — MEDIUM](#23-matching-engine--medium)
- [3. Feature Dependencies](#3-feature-dependencies)

<!-- /toc -->

---

## 1. Overview

The Identity Resolution DESIGN is decomposed into three features aligned to the implementation phases defined in the PRD. Each feature builds on the previous, delivering incremental value while maintaining a working system at each step.

**Decomposition Strategy**:
- Features grouped by **implementation phase**: seed → bootstrap → matching. Each phase delivers independently testable capabilities.
- Feature 1 (Initial Seed) establishes the `aliases` table and resolution API — the minimum viable system where HR data is directly loaded.
- Feature 2 (Bootstrap Pipeline) introduces the `identity_inputs` ingestion mechanism, BootstrapJob processing, and conflict/unmapped tracking — enabling automated alias creation from connector data.
- Feature 3 (Matching Engine) adds configurable matching rules with three-phase evaluation (B1/B2/B3), confidence scoring, and operator workflows — enabling intelligent alias resolution beyond exact matches.
- Dependencies are linear: Feature 1 → Feature 2 → Feature 3. No circular dependencies.
- 100% coverage of all DESIGN components, tables, and sequences verified.

**Late-Phase Items (Future Scope)**:
The following capabilities are defined in the PRD (p3 priority) and DESIGN but are not decomposed into features in this release:
- **Merge/split operations**: `merge_audits` table, merge/split API endpoints, `cpt-insightspec-ir-seq-merge` sequence. PRD FRs: `cpt-ir-fr-merge`, `cpt-ir-fr-split`, `cpt-ir-fr-merge-audit`, `cpt-ir-fr-idempotent-mutations`. NFR: `cpt-ir-nfr-merge-reversibility`.
- **GDPR alias deletion**: `alias_gdpr_deleted` table, purge API endpoint. PRD FRs: `cpt-ir-fr-gdpr-purge`. NFR: `cpt-ir-nfr-gdpr-erasure`.
- **Operator API endpoints** for merge/split/purge: These will be added when late-phase features are planned.

These items have schema defined in DESIGN §3.7 (`cpt-insightspec-ir-dbtable-merge-audits`, `cpt-insightspec-ir-dbtable-alias-gdpr-deleted`) for forward reference. Implementation will be planned in a separate DECOMPOSITION cycle.

---

## 2. Entries

**Overall implementation status:**

- [ ] `p1` - **ID**: `cpt-ir-status-overall`

### 2.1 [Initial Seed](feature-initial-seed/) — HIGH

- [ ] `p1` - **ID**: `cpt-ir-feature-initial-seed`

- **Purpose**: Establish the `aliases` table and Resolution API to enable cross-platform analytics from day one. HR Bronze data is loaded directly into `persons` (person domain) and `aliases` via dbt seed models, providing the minimum viable identity resolution: every Gold analytics query can resolve `person_id` for HR-sourced aliases.

- **Depends On**: None

- **Scope**:
  - Create `aliases` table in ClickHouse with PR #55 schema
  - dbt seed models to load HR Bronze data (BambooHR employees) into `aliases`
  - Resolution API: `POST /resolve`, `POST /batch-resolve`
  - Hot-path alias lookup in `aliases` table
  - ClickHouse Dictionary for analytical Silver step 2 enrichment
  - Tenant isolation on all queries
  - Cross-domain integration: `aliases.person_id` references `persons.person_id` (person domain creates person records; identity-resolution seeds the initial `persons` from `identity_inputs`, see ADR-0002)

- **Out of scope**:
  - `identity_inputs` table (Feature 2)
  - BootstrapJob incremental processing (Feature 2)
  - Match rules and MatchingEngine (Feature 3)
  - Unmapped queue (Feature 2)
  - Conflict detection (Feature 2)
  - Merge/split operations (late phase)
  - GDPR deletion (late phase)

- **Requirements Covered**:

  - [ ] `p1` - `cpt-ir-fr-seed-aliases`
  - [ ] `p1` - `cpt-ir-fr-resolve-alias`
  - [ ] `p1` - `cpt-ir-fr-batch-resolve`
  - [ ] `p1` - `cpt-ir-fr-tenant-isolation`
  - [ ] `p1` - `cpt-ir-nfr-alias-lookup-latency`
  - [ ] `p1` - `cpt-ir-nfr-tenant-isolation`

- **Design Principles Covered**:

  - [ ] `p2` - `cpt-insightspec-ir-principle-alias-centric`
  - [ ] `p2` - `cpt-insightspec-ir-principle-ch-native`
  - [ ] `p2` - `cpt-insightspec-ir-principle-domain-isolation`

- **Design Constraints Covered**:

  - [ ] `p2` - `cpt-insightspec-ir-constraint-ch-only`
  - [ ] `p2` - `cpt-insightspec-ir-constraint-naming`
  - [ ] `p2` - `cpt-insightspec-ir-constraint-domain-boundary`
  - [ ] `p2` - `cpt-insightspec-ir-constraint-half-open-intervals`

- **Domain Model Entities**:
  - `aliases` (create — primary table for this feature)
  - `persons` (cross-domain reference — created by person domain dbt seed)

- **Design Components**:

  - [ ] `p2` - `cpt-insightspec-ir-component-resolution-service`

- **API**:
  - POST /api/identity/resolve
  - POST /api/identity/batch-resolve

- **Sequences**:

  - [ ] `p1` - `cpt-insightspec-ir-seq-resolve-hot`

- **Data**:

  - [ ] `p3` - `cpt-insightspec-ir-db-schemas`
  - [ ] `p1` - `cpt-insightspec-ir-dbtable-aliases`
  - [x] `p1` - `cpt-insightspec-ir-dbtable-persons-mariadb`

- **Interfaces**:

  - [ ] `p1` - `cpt-ir-interface-resolution-api`
  - [ ] `p1` - `cpt-ir-contract-person-domain`

---

### 2.2 [Bootstrap Pipeline](feature-bootstrap-pipeline/) — HIGH

- [ ] `p1` - **ID**: `cpt-ir-feature-bootstrap-pipeline`

- **Purpose**: Enable automated, incremental alias creation from connector data. Connectors write alias observations to `identity_inputs`; the BootstrapJob processes them into the `aliases` table, routing unresolvable aliases to the `unmapped` queue and detecting alias-level conflicts. This replaces the one-time dbt seed with a continuous pipeline that handles new connectors and ongoing syncs.

- **Depends On**: `cpt-ir-feature-initial-seed` (aliases table and Resolution API must exist)

- **Scope**:
  - Create `identity_inputs` table in ClickHouse
  - Create `unmapped` table for unresolved aliases
  - Create `conflicts` table for alias-level disagreements
  - BootstrapJob: reads identity_inputs incrementally (`_synced_at > last_watermark`). See DESIGN §5 REC-IR-02 for recommended watermark mechanism (dbt incremental + `bootstrap_watermarks` table)
  - Alias normalization: email/username → `lower(trim())`; others → `trim()`
  - Auto-create alias on exact match (confidence >= 1.0 from direct lookup)
  - Route unresolved aliases to `unmapped` table
  - Detect alias conflicts when same alias claimed by different persons
  - Track `last_observed_at` for existing aliases
  - Auto-resolve unmapped entries when matching aliases are created
  - Idempotent bootstrap runs (dedup on natural key)
  - Argo Workflow integration for scheduling

- **Out of scope**:
  - Configurable match rules (Feature 3 — bootstrap uses direct lookup only in this feature)
  - Fuzzy matching (Feature 3)
  - Operator unmapped queue management UI (Feature 3)
  - Merge/split (late phase)
  - GDPR deletion (late phase)

- **Requirements Covered**:

  - [x] `p1` - `cpt-ir-fr-accept-bootstrap-inputs`
  - [x] `p1` - `cpt-ir-fr-bootstrap-incremental`
  - [ ] `p1` - `cpt-ir-fr-normalize-aliases`
  - [ ] `p1` - `cpt-ir-fr-create-alias-exact`
  - [ ] `p1` - `cpt-ir-fr-route-unmapped`
  - [ ] `p1` - `cpt-ir-fr-track-observations`
  - [ ] `p1` - `cpt-ir-fr-bootstrap-idempotent`
  - [ ] `p2` - `cpt-ir-fr-alias-conflict-detection`
  - [ ] `p2` - `cpt-ir-fr-auto-resolve-unmapped`
  - [ ] `p1` - `cpt-ir-nfr-bootstrap-throughput`
  - [ ] `p1` - `cpt-ir-nfr-bootstrap-idempotency`

- **Design Principles Covered**:

  - [ ] `p2` - `cpt-insightspec-ir-principle-fail-safe`

- **Design Constraints Covered**:

  (Inherits all constraints from Feature 1)

- **Domain Model Entities**:
  - `identity_inputs` (create)
  - `aliases` (update — add new aliases from bootstrap)
  - `unmapped` (create)
  - `conflicts` (create)

- **Design Components**:

  - [ ] `p2` - `cpt-insightspec-ir-component-bootstrap-job`
  - [ ] `p2` - `cpt-insightspec-ir-component-conflict-detector`

- **API**:
  - (No new API endpoints — BootstrapJob is a batch job, not an API service)
  - Connector write contract: dbt `identity_inputs_from_history` macro applied to `fields_history` models (implemented for BambooHR and Zoom)

- **Sequences**:

  - [ ] `p1` - `cpt-insightspec-ir-seq-bootstrap-processing`

- **Data**:

  - [x] `p1` - `cpt-insightspec-ir-dbtable-identity-inputs`
  - [ ] `p2` - `cpt-insightspec-ir-dbtable-unmapped`
  - [ ] `p2` - `cpt-insightspec-ir-dbtable-conflicts`

- **Interfaces**:

  - [x] `p1` - `cpt-ir-contract-bootstrap-inputs`

---

### 2.3 [Matching Engine](feature-matching-engine/) — MEDIUM

- [ ] `p2` - **ID**: `cpt-ir-feature-matching-engine`

- **Purpose**: Enable intelligent alias resolution beyond exact matches. Configurable match rules evaluate candidates using three-phase scoring (B1 deterministic, B2 normalization/cross-system, B3 fuzzy). Integrates with BootstrapJob for cold-path evaluation and provides operator workflows for unmapped queue management and manual alias CRUD.

- **Depends On**: `cpt-ir-feature-bootstrap-pipeline` (BootstrapJob must exist to invoke MatchingEngine on cold path; unmapped table must exist for suggestions)

- **Scope**:
  - Create `match_rules` table with seed data for B1/B2/B3 rules
  - MatchingEngine component: loads rules, evaluates against candidates, computes composite confidence
  - Three-phase pipeline: B1 (exact email, exact HR ID), B2 (case-insensitive email, domain alias, cross-system username), B3 (Jaro-Winkler, Soundex)
  - Confidence thresholds: >= 1.0 auto-link, 0.50-0.99 suggestion, < 0.50 unmapped
  - Fuzzy rules disabled by default; NEVER auto-link
  - Integration with BootstrapJob cold path: when direct lookup fails, invoke MatchingEngine
  - Integration with ResolutionService cold path: `POST /resolve` falls through to MatchingEngine
  - Operator API: `GET /unmapped`, `POST /unmapped/:id/resolve`, `POST /unmapped/:id/ignore`
  - Operator API: `GET /rules`, `PUT /rules/:id`
  - Operator API: `GET /persons/:id/aliases`, `POST /persons/:id/aliases`, `DELETE /persons/:id/aliases/:alias_id`
  - ClickHouse Dictionary for analytical alias lookup

- **Out of scope**:
  - Merge/split operations (late phase)
  - GDPR deletion (late phase)

- **Requirements Covered**:

  - [ ] `p2` - `cpt-ir-fr-configurable-rules`
  - [ ] `p2` - `cpt-ir-fr-three-phase-matching`
  - [ ] `p2` - `cpt-ir-fr-no-fuzzy-autolink`
  - [ ] `p2` - `cpt-ir-fr-unmapped-management`
  - [ ] `p2` - `cpt-ir-fr-manual-alias-crud`
  - [ ] `p2` - `cpt-ir-nfr-no-fuzzy-autolink`

- **Design Principles Covered**:

  - [ ] `p2` - `cpt-insightspec-ir-principle-conservative-matching`

- **Design Constraints Covered**:

  - [ ] `p2` - `cpt-insightspec-ir-constraint-no-fuzzy-autolink`

- **Domain Model Entities**:
  - `match_rules` (create + seed default rules)
  - `unmapped` (update — add suggestions from MatchingEngine)
  - `aliases` (update — auto-link from MatchingEngine results)

- **Design Components**:

  - [ ] `p2` - `cpt-insightspec-ir-component-matching-engine`

- **API**:
  - GET /api/identity/unmapped
  - POST /api/identity/unmapped/:id/resolve
  - POST /api/identity/unmapped/:id/ignore
  - GET /api/identity/rules
  - PUT /api/identity/rules/:id
  - GET /api/identity/persons/:id/aliases
  - POST /api/identity/persons/:id/aliases
  - DELETE /api/identity/persons/:id/aliases/:alias_id

- **Sequences**:

  (MatchingEngine is invoked within `cpt-insightspec-ir-seq-bootstrap-processing` and `cpt-insightspec-ir-seq-resolve-hot` — both already assigned to Features 1 and 2. No new sequences unique to this feature.)

- **Data**:

  - [ ] `p2` - `cpt-insightspec-ir-dbtable-match-rules`

- **Interfaces**:

  - [ ] `p2` - `cpt-ir-interface-ch-dictionary`

---

## 3. Feature Dependencies

```text
cpt-ir-feature-initial-seed (HIGH, p1)
    |
    +---> cpt-ir-feature-bootstrap-pipeline (HIGH, p1)
              |
              +---> cpt-ir-feature-matching-engine (MEDIUM, p2)
```

**Late-phase items (not yet decomposed):**
```text
cpt-ir-feature-matching-engine
    |
    +---> [future] merge/split operations (p3)
    +---> [future] GDPR alias deletion (p3)
    +---> [future] operator merge/split/purge API (p3)
```

**Dependency Rationale**:

- `cpt-ir-feature-bootstrap-pipeline` requires `cpt-ir-feature-initial-seed`: The `aliases` table and Resolution API must exist before the BootstrapJob can create/update alias records and invoke resolution lookups. The dbt seed provides the initial person+alias foundation that bootstrap extends.

- `cpt-ir-feature-matching-engine` requires `cpt-ir-feature-bootstrap-pipeline`: The MatchingEngine is invoked by the BootstrapJob on the cold path (when direct alias lookup fails). The `unmapped` table must exist for the MatchingEngine to write suggestions. Without the bootstrap pipeline, there is no invocation path for the MatchingEngine.

**Coverage Verification**:

| DESIGN Element | Feature |
|---|---|
| `cpt-insightspec-ir-component-resolution-service` | Feature 1 (initial-seed) |
| `cpt-insightspec-ir-component-bootstrap-job` | Feature 2 (bootstrap-pipeline) |
| `cpt-insightspec-ir-component-conflict-detector` | Feature 2 (bootstrap-pipeline) |
| `cpt-insightspec-ir-component-matching-engine` | Feature 3 (matching-engine) |
| `cpt-insightspec-ir-dbtable-aliases` | Feature 1 (initial-seed) |
| `cpt-insightspec-ir-dbtable-identity-inputs` | Feature 2 (bootstrap-pipeline) |
| `cpt-insightspec-ir-dbtable-persons-mariadb` | Feature 1 (initial-seed) |
| `cpt-insightspec-ir-dbtable-unmapped` | Feature 2 (bootstrap-pipeline) |
| `cpt-insightspec-ir-dbtable-conflicts` | Feature 2 (bootstrap-pipeline) |
| `cpt-insightspec-ir-dbtable-match-rules` | Feature 3 (matching-engine) |
| `cpt-insightspec-ir-dbtable-merge-audits` | Late phase (future) |
| `cpt-insightspec-ir-dbtable-alias-gdpr-deleted` | Late phase (future) |
| `cpt-insightspec-ir-seq-resolve-hot` | Feature 1 (initial-seed) |
| `cpt-insightspec-ir-seq-bootstrap-processing` | Feature 2 (bootstrap-pipeline) |
| `cpt-insightspec-ir-seq-merge` | Late phase (future) |
| `cpt-insightspec-ir-interface-api` | Feature 1 (initial-seed) |

| PRD Requirement | Feature |
|---|---|
| `cpt-ir-fr-seed-aliases` (p1) | Feature 1 |
| `cpt-ir-fr-resolve-alias` (p1) | Feature 1 |
| `cpt-ir-fr-batch-resolve` (p1) | Feature 1 |
| `cpt-ir-fr-tenant-isolation` (p1) | Feature 1 |
| `cpt-ir-fr-accept-bootstrap-inputs` (p1) | Feature 2 |
| `cpt-ir-fr-bootstrap-incremental` (p1) | Feature 2 |
| `cpt-ir-fr-normalize-aliases` (p1) | Feature 2 |
| `cpt-ir-fr-create-alias-exact` (p1) | Feature 2 |
| `cpt-ir-fr-route-unmapped` (p1) | Feature 2 |
| `cpt-ir-fr-track-observations` (p1) | Feature 2 |
| `cpt-ir-fr-bootstrap-idempotent` (p1) | Feature 2 |
| `cpt-ir-fr-alias-conflict-detection` (p2) | Feature 2 |
| `cpt-ir-fr-auto-resolve-unmapped` (p2) | Feature 2 |
| `cpt-ir-fr-configurable-rules` (p2) | Feature 3 |
| `cpt-ir-fr-three-phase-matching` (p2) | Feature 3 |
| `cpt-ir-fr-no-fuzzy-autolink` (p2) | Feature 3 |
| `cpt-ir-fr-unmapped-management` (p2) | Feature 3 |
| `cpt-ir-fr-manual-alias-crud` (p2) | Feature 3 |
| `cpt-ir-fr-merge` (p3) | Late phase |
| `cpt-ir-fr-split` (p3) | Late phase |
| `cpt-ir-fr-merge-audit` (p3) | Late phase |
| `cpt-ir-fr-gdpr-purge` (p3) | Late phase |
| `cpt-ir-fr-idempotent-mutations` (p3) | Late phase |
