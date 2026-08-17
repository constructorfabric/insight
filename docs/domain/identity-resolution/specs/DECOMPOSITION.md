# Decomposition: Identity Resolution

<!-- toc -->

- [1. Overview](#1-overview)
- [2. Entries](#2-entries)
  - [2.1 Identity Store & Read Paths — HIGH](#21-identity-store--read-paths--high)
  - [2.2 Evidence Intake & Seed Fold — HIGH](#22-evidence-intake--seed-fold--high)
  - [2.3 Matching Engine — MEDIUM](#23-matching-engine--medium)
  - [2.4 Manual Resolution — HIGH](#24-manual-resolution--high)
- [3. Feature Dependencies](#3-feature-dependencies)

<!-- /toc -->

---

## 1. Overview

The Identity Resolution DESIGN is decomposed into four features. Features 1–2 cover the implemented journal architecture (DESIGN v3.0); Feature 3 is the future matcher; Feature 4 is the manual-resolution capability of the current iteration.

> **History note**: Features 1–2 were originally planned around an alias-table pipeline (`aliases` store, BootstrapJob, `unmapped`/`conflicts` queues, resolve API, ClickHouse Dictionary). That plan was not built; the entries below describe what shipped instead, and the corresponding alias-pipeline requirements are marked superseded in the PRD. The historical entries remain in this file's git history.

**Decomposition Strategy**:
- Feature 1 (Identity Store & Read Paths) establishes the `persons` journal with stable `person_id` (ADR-0002), its derived caches, and both read paths — the service read API and the analytics mirror + resolve macro.
- Feature 2 (Evidence Intake & Seed Fold) introduces the `identity_inputs` evidence contract and the scheduled persons-seed fold that binds accounts to persons, plus the persons-sync mirror publisher.
- Feature 3 (Matching Engine, future) adds configurable matching rules with confidence-scored proposals — never auto-applied.
- Feature 4 (Manual Resolution) adds operator correction verbs over the journal (ADR-0003) with the resolver upgrade and seed hardening they require.
- Dependencies: Feature 1 → Feature 2 → Feature 3, and Feature 4 → Feature 3 (proposal acceptance flows through the Feature 4 operator API); Feature 1 → Feature 4. No circular dependencies.
- 100% coverage of all DESIGN components, tables, and sequences verified.

**Manual Resolution (current iteration)**: operator corrections were re-scoped from "late phase" to `p1` by [ADR-0003](ADR/0003-operator-decisions-as-persons-observations.md) — the journal-based model replaced the snapshot-based merge/split plan, and the v1 merge/split/audit/idempotency requirements were superseded by their `-v2` forms in the PRD. Entry 2.4 below tracks this feature; its FEATURE.md follows with the implementation PR (design reviewed in constructorfabric/insight#2180).

**Late-Phase Items (Future Scope)**:
- **GDPR alias deletion**: erasure archive table and purge flow. PRD FRs: `cpt-ir-fr-gdpr-purge`. NFR: `cpt-ir-nfr-gdpr-erasure`. Schema retained as a future-table summary in DESIGN §3.7; implementation will be planned in a separate DECOMPOSITION cycle.
- **Admin identity console**: a required product surface per the umbrella epic (#1873) — scheduled as the next step once the Feature 4 API stabilizes; not decomposed here.

---

## 2. Entries

**Overall implementation status:**

- [ ] `p1` - **ID**: `cpt-ir-status-overall`

### 2.1 Identity Store & Read Paths — HIGH

- [ ] `p1` - **ID**: `cpt-ir-feature-initial-seed`

- **Purpose**: Establish the `persons` observation journal as the source of truth for the account-to-person binding (stable random `person_id`, ADR-0002) and the two read paths every consumer uses: the identity-resolution service read API (request-time person lookups) and the analytics mirror + `resolve_person_id` macro (build-time resolution for gold).

- **Depends On**: None

- **Scope**:
  - `persons` journal and `org_chart` in MariaDB, schema owned by the service (SeaORM migrations, ADR-0006)
  - Service read API: person profile and visibility lookups over the journal (component spec)
  - Analytics read path: `identity.identity_persons` mirror + build-time resolution through the resolve macro
  - Tenant isolation on all queries (see the known gap on the evidence read path in the PRD)

- **Out of scope**:
  - `identity_inputs` evidence contract and the seed fold (Feature 2)
  - Match rules and MatchingEngine (Feature 3)
  - Operator corrections (Feature 4)
  - GDPR deletion (late phase)

- **Requirements Covered**:

  - [ ] `p1` - `cpt-ir-fr-seed-aliases` (superseded — legacy dbt seeds)
  - [ ] `p1` - `cpt-ir-fr-resolve-alias` (superseded)
  - [ ] `p1` - `cpt-ir-fr-batch-resolve` (superseded)
  - [ ] `p1` - `cpt-ir-fr-tenant-isolation`
  - [ ] `p1` - `cpt-ir-nfr-alias-lookup-latency`
  - [ ] `p1` - `cpt-ir-nfr-tenant-isolation`

- **Design Principles Covered**:

  - [ ] `p2` - `cpt-insightspec-ir-principle-alias-centric` (v1 framing — retained; see the storage-split principle)
  - [ ] `p2` - `cpt-insightspec-ir-principle-ch-native-v2`
  - [ ] `p2` - `cpt-insightspec-ir-principle-domain-isolation`

- **Design Constraints Covered**:

  - [ ] `p2` - `cpt-insightspec-ir-constraint-storage-split-v2`
  - [ ] `p2` - `cpt-insightspec-ir-constraint-naming`
  - [ ] `p2` - `cpt-insightspec-ir-constraint-domain-boundary`
  - [ ] `p2` - `cpt-insightspec-ir-constraint-half-open-intervals`

- **Domain Model Entities**:
  - `persons` (journal — seeded by this domain per ADR-0002)

- **Design Components**:

  - [x] `p1` - `cpt-insightspec-ir-component-identity-read-api`

- **API**:
  - Person lookup endpoints of the identity-resolution service (see component spec)

- **Sequences**:

  - `cpt-insightspec-ir-seq-build-resolution`

- **Data**:

  - [ ] `p3` - `cpt-insightspec-ir-db-schemas`
  - `cpt-insightspec-ir-dbtable-persons-mariadb`

- **Interfaces**:

  - [ ] `p1` - `cpt-ir-interface-resolution-api`
  - [ ] `p1` - `cpt-ir-contract-person-domain`

---

### 2.2 Evidence Intake & Seed Fold — HIGH

- [ ] `p1` - **ID**: `cpt-ir-feature-bootstrap-pipeline`

- **Purpose**: Give connectors one uniform write target for identity observations and fold that evidence into the journal automatically. Connectors populate `identity_inputs` through dbt (`identity_inputs_from_history` macro, incremental append models); the scheduled persons-seed groups accounts by e-mail and binds them (reuse / link-by-e-mail / mint / skip, never merging existing persons); persons-sync republishes the journal to ClickHouse for analytics.

- **Depends On**: `cpt-ir-feature-initial-seed` (the `persons` journal and read paths must exist)

- **Scope**:
  - `identity_inputs` table and write contract; per-connector dbt models via the shared macro (incremental `append` on `_synced_at`)
  - persons-seed as a scheduled service subcommand: run-lock, input guards with explicit `--force`, run journal in `operations` with per-branch counters
  - Seed fold semantics per DESIGN §3.2 PersonsSeed (including the documented divergent-group known gap, addressed in Feature 4)
  - persons-sync: atomic republish of the journal into `identity.identity_persons`
  - Idempotent re-runs (journal natural key; deterministic `created_at` from `_synced_at`)

- **Out of scope**:
  - Incremental watermark processing — each run currently folds the full evidence set (`cpt-ir-fr-bootstrap-incremental` open; DESIGN §5 REC-IR-02)
  - Configurable match rules and fuzzy matching (Feature 3)
  - Operator corrections and seed hardening (Feature 4)
  - GDPR deletion (late phase)

- **Requirements Covered**:

  - [x] `p1` - `cpt-ir-fr-accept-bootstrap-inputs`
  - [ ] `p1` - `cpt-ir-fr-bootstrap-incremental` (open: full-set fold today; watermark per REC-IR-02)
  - [ ] `p1` - `cpt-ir-fr-normalize-aliases` (superseded)
  - [ ] `p1` - `cpt-ir-fr-create-alias-exact` (superseded)
  - [ ] `p3` - `cpt-ir-fr-route-unmapped` (future — with the matcher)
  - [ ] `p1` - `cpt-ir-fr-track-observations` (superseded)
  - [ ] `p1` - `cpt-ir-fr-bootstrap-idempotent` (superseded)
  - [ ] `p2` - `cpt-ir-fr-alias-conflict-detection` (superseded)
  - [ ] `p2` - `cpt-ir-fr-auto-resolve-unmapped` (future — with the matcher)
  - [ ] `p1` - `cpt-ir-nfr-bootstrap-throughput`
  - [ ] `p1` - `cpt-ir-nfr-bootstrap-idempotency`

- **Design Principles Covered**:

  - [ ] `p2` - `cpt-insightspec-ir-principle-fail-safe`

- **Design Constraints Covered**:

  (Inherits all constraints from Feature 1)

- **Domain Model Entities**:
  - `identity_inputs` (create)
  - `persons` (append observations per ADR-0002 — never updated)
  - `unmapped` / `conflicts` (future tables — the v1 review queue is derived; see DESIGN §3.7)

- **Design Components**:

  - [x] `p1` - `cpt-insightspec-ir-component-persons-seed`
  - [x] `p1` - `cpt-insightspec-ir-component-persons-sync`

- **API**:
  - (No new API endpoints — the seed is a scheduled job, not an API service)
  - Connector write contract: dbt `identity_inputs_from_history` macro applied to `fields_history` models (implemented for BambooHR and Zoom)

- **Sequences**:

  - `cpt-insightspec-ir-seq-seed-run`

- **Data**:

  - `cpt-insightspec-ir-dbtable-identity-inputs`

- **Interfaces**:

  - [x] `p1` - `cpt-ir-contract-bootstrap-inputs`

---

### 2.3 Matching Engine — MEDIUM

- [ ] `p2` - **ID**: `cpt-ir-feature-matching-engine`

- **Purpose** (future): Enable matching beyond the seed's exact-e-mail link. Configurable match rules evaluate candidates using three-phase scoring (B1 deterministic, B2 normalization/cross-system, B3 fuzzy) and produce confidence-scored **proposals** for operator review. Two ADR-0003 invariants bind this feature: the matcher never writes bindings (acceptance is an operator act through the Feature 4 API), and operator decisions in the journal override any rule.

- **Depends On**: `cpt-ir-feature-bootstrap-pipeline` (evidence stream and journal), `cpt-ir-feature-manual-resolution` (the accept/reject surface for proposals)

- **Scope**:
  - `match_rules` storage with seed data for B1/B2/B3 rules (future table — DESIGN §3.7)
  - MatchingEngine component: loads rules, evaluates candidates over evidence and journal, computes composite confidence
  - Three-phase pipeline: B1 (exact e-mail, exact HR ID), B2 (case-insensitive e-mail, domain alias, cross-system username), B3 (Jaro-Winkler, Soundex)
  - Confidence thresholds ordering proposals; fuzzy rules disabled by default and NEVER auto-link
  - Proposal review integration with the operator resolution API (accept / reject / defer)
  - Rule configuration surface for operators

- **Out of scope**:
  - Any automatic application of proposals (structurally excluded — ADR-0003)
  - GDPR deletion (late phase)

- **Requirements Covered**:

  - [ ] `p2` - `cpt-ir-fr-configurable-rules`
  - [ ] `p2` - `cpt-ir-fr-three-phase-matching`
  - [ ] `p2` - `cpt-ir-fr-no-fuzzy-autolink`
  - [ ] `p2` - `cpt-ir-fr-unmapped-management`
  - [ ] `p2` - `cpt-ir-fr-manual-alias-crud` (superseded)
  - [ ] `p2` - `cpt-ir-nfr-no-fuzzy-autolink`

- **Design Principles Covered**:

  - [ ] `p2` - `cpt-insightspec-ir-principle-conservative-matching`

- **Design Constraints Covered**:

  - [ ] `p2` - `cpt-insightspec-ir-constraint-no-fuzzy-autolink`

- **Domain Model Entities**:
  - `match_rules` (create + seed default rules — future table, DESIGN §3.7)
  - Proposal storage or derivation (design decision of this feature)
  - `persons` journal (read-only for the matcher; bindings are written only by operator acceptance through the Feature 4 API)

- **Design Components**:

  - [ ] `p2` - `cpt-insightspec-ir-component-matching-engine`

- **API**:
  - Match-rule configuration and proposal review endpoints (to be designed with this feature; proposals are accepted through the Feature 4 operator surface)

- **Sequences**:

  (The MatchingEngine is invoked within the seed-run and build-resolution sequences already assigned to Features 1 and 2. No new sequences unique to this feature.)

- **Data**:
  - `match_rules` (future table — summary in DESIGN §3.7)

- **Interfaces**:
  - Match-rule configuration API (future; will be specified with this feature)

---

### 2.4 Manual Resolution — HIGH

- [ ] `p1` - **ID**: `cpt-ir-feature-manual-resolution`

- **Purpose**: Give the operator correction verbs over the account-to-person binding — bind (single/bulk), merge, detach, exclude — plus the derived review queue and binding history, per [ADR-0003](ADR/0003-operator-decisions-as-persons-observations.md). Corrections are appended to the `persons` journal and survive every seed re-run. Reviewed design with scenarios: constructorfabric/insight#2180. FEATURE.md to be authored with the implementation PR.

- **Depends On**: `cpt-ir-feature-initial-seed` (the `persons` journal, seed, and read API must exist)

- **Scope**:
  - Operator write verbs appending binding observations authored by the operator
  - Derived review queue (accounts pending a decision, contested-binding groups not explained by an operator decision, and no-evidence accounts surfaced from `identity_inputs`) with candidates and counts
  - Resolution-rate reporting (bound / pending / no-evidence / excluded shares — the operator-visible match rate of the umbrella epic)
  - Per-account binding history (explain) and per-person account listing (matching table)
  - Seed hardening: per-account bindings win over group collapse (removing the path that can silently re-derive a binding); author-aware conflict classification (bindings loader returns author); contested e-mails stop auto-linking
  - Evidence reader fix: honor empty-value DELETE (closure) rows — the current non-empty filter drops them, leaving tombstones inert; required for correct seed closure handling and for the queue's UPSERT/DELETE fold
  - Reserved excluded-person sentinel treated as "no person" by every consumer — resolve macro (NULL), service read API, person domain, review queue (normative definition in DESIGN par. 4.3)
  - dbt resolver upgrade: account-first person resolution (latest `value_type='id'` binding per source account) with e-mail fallback; contested e-mail resolves to NULL — required for corrections to reach gold (DESIGN par. 4.4)
  - Decision-aware API idempotency + unique per-row observation timestamps (natural key has no account discriminator — DESIGN par. 3.7 index note)
  - Account-derived e-mail fallback in the resolver: `identity_inputs` provides the account-to-value linkage (`source_account_id` on every row); an e-mail resolves through the current bindings of its observing accounts

- **Out of scope**:
  - Stored negative rules, value blocklists, proposals with confidence, automatic revert, multi-operator concurrency — deferred with explicit triggers (ADR-0003)
  - Ignore/defer (snooze) for queue items — deliberate narrowing; returns with the proposal store
  - GDPR deletion (late phase)

- **Requirements Covered**:

  - [ ] `p1` - `cpt-ir-fr-merge-v2`
  - [ ] `p1` - `cpt-ir-fr-split-v2`
  - [ ] `p1` - `cpt-ir-fr-operator-bind`
  - [ ] `p1` - `cpt-ir-fr-operator-exclude`
  - [ ] `p1` - `cpt-ir-fr-review-queue`
  - [ ] `p1` - `cpt-ir-fr-correction-durability`
  - [ ] `p1` - `cpt-ir-fr-merge-audit-v2`
  - [ ] `p1` - `cpt-ir-fr-idempotent-mutations-v2`
  - [ ] `p2` - `cpt-ir-fr-binding-history`
  - [ ] `p1` - `cpt-ir-nfr-merge-reversibility`

- **Design Principles Covered**:

  - [ ] `p1` - `cpt-insightspec-ir-principle-append-only-journal`
  - [ ] `p2` - `cpt-insightspec-ir-principle-fail-safe`

- **Design Constraints Covered**:

  (Inherits all constraints from Feature 1)

- **Domain Model Entities**:
  - `persons` (append operator corrections)
  - `operations` (journal operator calls)

- **Design Components**:

  - [ ] `p1` - `cpt-insightspec-ir-component-operator-resolution-api`

- **API**:
  - Operator resolution endpoints (working shapes in DESIGN §3.3; contracts fixed at FEATURE level)

- **Sequences**:

  - `cpt-insightspec-ir-seq-operator-correction`

- **Interfaces**:

  - [ ] `p1` - `cpt-ir-interface-analytics-resolution`
  - [ ] `p1` - `cpt-insightspec-ir-interface-api-v2`

---

## 3. Feature Dependencies

```text
cpt-ir-feature-initial-seed (HIGH, p1)
    |
    +---> cpt-ir-feature-bootstrap-pipeline (HIGH, p1)
    |         |
    |         +---> cpt-ir-feature-matching-engine (MEDIUM, p2)
    |                   ^
    +---> cpt-ir-feature-manual-resolution (HIGH, p1)
                        (proposal acceptance surface)
```

**Late-phase items (not yet decomposed):**
```text
cpt-ir-feature-manual-resolution
    |
    +---> [future] stored negative rules / value blocklists (with the matcher)
    +---> [future] GDPR alias deletion (p3)
```

**Dependency Rationale**:

- `cpt-ir-feature-bootstrap-pipeline` requires `cpt-ir-feature-initial-seed`: the `persons` journal, its schema ownership, and the read paths must exist before the seed can fold evidence into them and persons-sync can publish the result.

- `cpt-ir-feature-matching-engine` requires `cpt-ir-feature-bootstrap-pipeline`: the matcher consumes the evidence stream and the journal produced by the intake/fold pipeline, and its proposals are only meaningful once automatic binding runs continuously.

- `cpt-ir-feature-manual-resolution` requires `cpt-ir-feature-initial-seed`: correction verbs append to the journal and are served by the same service; its resolver upgrade extends the analytics read path established there.

**Coverage Verification**:

| DESIGN Element | Feature |
|---|---|
| `cpt-insightspec-ir-component-identity-read-api` | Feature 1 (initial-seed) |
| `cpt-insightspec-ir-component-persons-seed` | Feature 2 (bootstrap-pipeline) |
| `cpt-insightspec-ir-component-persons-sync` | Feature 2 (bootstrap-pipeline) |
| `cpt-insightspec-ir-component-matching-engine` | Feature 3 (matching-engine) |
| `cpt-insightspec-ir-component-operator-resolution-api` | Feature 4 (manual-resolution) |
| `cpt-insightspec-ir-dbtable-identity-inputs` | Feature 2 (bootstrap-pipeline) |
| `cpt-insightspec-ir-dbtable-persons-mariadb` | Feature 1 (initial-seed) |
| `cpt-insightspec-ir-seq-build-resolution` | Feature 1 (initial-seed) |
| `cpt-insightspec-ir-seq-seed-run` | Feature 2 (bootstrap-pipeline) |
| `cpt-insightspec-ir-seq-operator-correction` | Feature 4 (manual-resolution) |
| `cpt-insightspec-ir-interface-api-v2` | Feature 4 (manual-resolution) |
| future tables (`match_rules`, `unmapped`, `conflicts`, `merge_audits`, `alias_gdpr_deleted`) | Future (DESIGN §3.7 summaries) |

| PRD Requirement | Feature |
|---|---|
| `cpt-ir-fr-seed-aliases` (p1, superseded — legacy dbt seeds) | Feature 1 |
| `cpt-ir-fr-resolve-alias` (p1, superseded) | Feature 1 |
| `cpt-ir-fr-batch-resolve` (p1, superseded) | Feature 1 |
| `cpt-ir-fr-tenant-isolation` (p1) | Feature 1 |
| `cpt-ir-fr-persons-history` (p1) | Feature 1 |
| `cpt-ir-fr-persons-initial-seed` (p1) | Feature 1 |
| `cpt-ir-fr-accept-bootstrap-inputs` (p1) | Feature 2 |
| `cpt-ir-fr-bootstrap-incremental` (p1) | Feature 2 |
| `cpt-ir-fr-normalize-aliases` (p1, superseded) | Feature 2 |
| `cpt-ir-fr-create-alias-exact` (p1, superseded) | Feature 2 |
| `cpt-ir-fr-route-unmapped` (p3, future) | Feature 3 |
| `cpt-ir-fr-track-observations` (p1, superseded) | Feature 2 |
| `cpt-ir-fr-bootstrap-idempotent` (p1, superseded) | Feature 2 |
| `cpt-ir-fr-alias-conflict-detection` (p2, superseded) | Feature 4 (derived queue) |
| `cpt-ir-fr-auto-resolve-unmapped` (p3, future) | Feature 3 |
| `cpt-ir-fr-configurable-rules` (p2) | Feature 3 |
| `cpt-ir-fr-three-phase-matching` (p2) | Feature 3 |
| `cpt-ir-fr-no-fuzzy-autolink` (p2) | Feature 3 |
| `cpt-ir-fr-unmapped-management` (p2, superseded) | Feature 3 → Feature 4 |
| `cpt-ir-fr-manual-alias-crud` (p2, superseded) | Feature 4 |
| `cpt-ir-fr-merge-v2` (p1) | Feature 4 |
| `cpt-ir-fr-split-v2` (p1) | Feature 4 |
| `cpt-ir-fr-operator-bind` (p1) | Feature 4 |
| `cpt-ir-fr-operator-exclude` (p1) | Feature 4 |
| `cpt-ir-fr-review-queue` (p1) | Feature 4 |
| `cpt-ir-fr-correction-durability` (p1) | Feature 4 |
| `cpt-ir-fr-merge-audit-v2` (p1) | Feature 4 |
| `cpt-ir-fr-idempotent-mutations-v2` (p1) | Feature 4 |
| `cpt-ir-fr-binding-history` (p2) | Feature 4 |
| `cpt-ir-fr-gdpr-purge` (p3) | Late phase |
