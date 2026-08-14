---
status: proposed
date: 2026-08-06
---

# Feature: Manual Identity Resolution (operator corrections)


<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Review the Queue and Bind an Account](#review-the-queue-and-bind-an-account)
  - [Merge Two Persons](#merge-two-persons)
  - [Detach an Account into a New Person](#detach-an-account-into-a-new-person)
  - [Exclude a Non-Person Account](#exclude-a-non-person-account)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [Apply One Decision](#apply-one-decision)
  - [Resolve an Addressed Account](#resolve-an-addressed-account)
  - [Claim an Observation Timestamp Slot](#claim-an-observation-timestamp-slot)
  - [Build the Review Queue](#build-the-review-queue)
  - [Resolve Person for Analytics (macro upgrade)](#resolve-person-for-analytics-macro-upgrade)
- [4. States (CDSL)](#4-states-cdsl)
  - [Queue Item Lifecycle](#queue-item-lifecycle)
- [5. Definitions of Done](#5-definitions-of-done)
  - [Correction Verbs](#correction-verbs)
  - [Decision-Aware Idempotency](#decision-aware-idempotency)
  - [Timestamp Uniqueness](#timestamp-uniqueness)
  - [Excluded Sentinel](#excluded-sentinel)
  - [Review Queue and Match Rate](#review-queue-and-match-rate)
  - [Binding History and Person Accounts](#binding-history-and-person-accounts)
  - [Operator Authorization](#operator-authorization)
  - [Analytics Resolver Upgrade](#analytics-resolver-upgrade)
- [6. Acceptance Criteria](#6-acceptance-criteria)

<!-- /toc -->

- [ ] `p1` - **ID**: `cpt-ir-featstatus-manual-resolution`
## 1. Feature Context

- [ ] `p1` - `cpt-ir-feature-manual-resolution`

### 1.1 Overview

Give an operator four correction verbs over the account-to-person binding — bind (single and bulk), merge, detach, exclude — plus the read surface that makes them usable: a review queue derived from evidence and current bindings, per-account binding history, and the accounts of one person. Every correction is appended to the `persons` journal and survives automatic re-runs.

The feature also ships the two enablers without which corrections are inert: the analytics resolver upgrade (corrections are `value_type='id'` bindings, while the v1 macro resolves by e-mail only) and the persons-seed hardening (the divergent-group collapse could re-derive an operator binding). Seed hardening landed first as its own change; this feature carries the API, the queue, and the resolver.

### 1.2 Purpose

Automatic resolution errs in both directions — under-merge (one human as two persons) and over-merge (two humans grouped through a shared value) — and the product has no supported way to change a binding once automation wrote it. This feature is the operator-driven correction surface decided in [ADR-0003](../ADR/0003-operator-decisions-as-persons-observations.md), reviewed in constructorfabric/insight#2180.

**Requirements**:

- `cpt-ir-fr-merge-v2`
- `cpt-ir-fr-split-v2`
- `cpt-ir-fr-operator-bind`
- `cpt-ir-fr-operator-exclude`
- `cpt-ir-fr-review-queue`
- `cpt-ir-fr-correction-durability`
- `cpt-ir-fr-merge-audit-v2`
- `cpt-ir-fr-idempotent-mutations-v2`
- `cpt-ir-fr-binding-history`
- `cpt-ir-nfr-merge-reversibility`

**Principles**:

- `cpt-insightspec-ir-principle-append-only-journal`
- `cpt-insightspec-ir-principle-fail-safe`

### 1.3 Actors

| Actor | Role in Feature |
|-------|-----------------|
| `cpt-ir-actor-operator` | Reviews the queue and issues corrections; authored rows carry their person id |
| `cpt-ir-actor-bootstrap-job` | Reuses operator bindings unchanged; surfaces what it cannot decide |
| `cpt-ir-actor-analytics-pipeline` | Resolves `person_id` at build time through the upgraded macro |

### 1.4 References

- **PRD**: [PRD.md](../PRD.md)
- **Design**: [DESIGN.md](../DESIGN.md)
- **Decisions**: [ADR-0003](../ADR/0003-operator-decisions-as-persons-observations.md), [ADR-0002](../ADR/0002-stable-person-id-via-persons-observations.md)
- **Dependencies**: `cpt-ir-feature-initial-seed` (journal, read paths). The seed hardening and the analytics resolver upgrade are parts of this feature, not external dependencies: corrections are inert without either.

## 2. Actor Flows (CDSL)

**Use cases**: `cpt-ir-usecase-review-unmapped`, `cpt-ir-usecase-merge`

### Review the Queue and Bind an Account

- [ ] `p1` - **ID**: `cpt-ir-flow-manual-resolution-review-and-bind`

**Actor**: `cpt-ir-actor-operator`

**Success Scenarios**:
- An account pending a decision is bound to the correct person and leaves the queue
- An account pending a decision is confirmed as its own person (bind to its current person) and leaves the queue
- A prepared matching table is imported in one call, with per-row outcomes

**Error Scenarios**:
- The caller lacks the operator grant
- One call names the same account twice — which person wins is the caller's contradiction, not the system's to guess
- A row addresses a value that resolves to more than one account, or to none
- A row names a person that does not exist

**Steps**:

> Every step below reads and writes within the caller's tenant: the tenant comes from the verified request context, never from the request body or path, so an operator cannot reach another tenant's accounts, persons or history.

1. [ ] - `p1` - Operator requests the review queue - `inst-mr-queue-request`
2. [ ] - `p1` - API: GET /v1/resolution/attention (items with candidates + resolution-rate shares) - `inst-mr-queue-api`
3. [ ] - `p1` - Operator inspects one account's history where needed - `inst-mr-history-inspect`
4. [ ] - `p1` - API: GET /v1/resolution/accounts/{source}/{source_id}/{account_id} (current binding + authored history) - `inst-mr-history-api`
5. [ ] - `p1` - Operator submits one or more bindings - `inst-mr-bind-submit`
6. [ ] - `p1` - API: POST /v1/resolution/bind (items addressed by account; addressing by observed value is reserved, see the addressing process) - `inst-mr-bind-api`
7. [ ] - `p1` - **IF** the caller has no operator grant - `inst-mr-authz-check`
   1. [ ] - `p1` - **RETURN** authorization error, nothing appended - `inst-mr-authz-reject`
8. [ ] - `p1` - **FOR EACH** item: resolve the target account, then apply `cpt-ir-algo-manual-resolution-apply-decision` - `inst-mr-bind-loop`
9. [ ] - `p1` - **RETURN** per-item outcomes (applied / no-op / skipped with reason) - `inst-mr-bind-return`
10. [ ] - `p1` - Queue item disappears on the next read because its condition no longer holds - `inst-mr-queue-dissolve`

### Merge Two Persons

- [ ] `p1` - **ID**: `cpt-ir-flow-manual-resolution-merge`

**Actor**: `cpt-ir-actor-operator`

**Success Scenarios**:
- Every account of the absorbed person is rebound to the survivor; history stays intact

**Error Scenarios**:
- Source and target are the same person
- Either person does not exist

**Steps**:
1. [ ] - `p1` - Operator submits the merge, naming the surviving person - `inst-mr-merge-submit`
2. [ ] - `p1` - API: POST /v1/resolution/merge (source_person_id, target_person_id, comment) - `inst-mr-merge-api`
3. [ ] - `p1` - **IF** source equals target or either person is unknown - `inst-mr-merge-validate`
   1. [ ] - `p1` - **RETURN** validation error, nothing appended - `inst-mr-merge-reject`
4. [ ] - `p1` - DB: SELECT persons (accounts currently bound to the source person) - `inst-mr-merge-collect`
5. [ ] - `p1` - **FOR EACH** account: apply `cpt-ir-algo-manual-resolution-apply-decision` with the survivor - `inst-mr-merge-loop`
6. [ ] - `p1` - **RETURN** the survivor and the number of rebound accounts - `inst-mr-merge-return`

### Detach an Account into a New Person

- [ ] `p1` - **ID**: `cpt-ir-flow-manual-resolution-detach`

**Actor**: `cpt-ir-actor-operator`

**Success Scenarios**:
- The account is rebound to a freshly minted person, regardless of how the current grouping arose

**Error Scenarios**:
- The account is unknown: nothing has observed it and it has no binding — detaching would mint a person for a typo

**Steps**:
1. [ ] - `p1` - Operator submits the detach for one account - `inst-mr-detach-submit`
2. [ ] - `p1` - API: POST /v1/resolution/detach (account) - `inst-mr-detach-api`
3. [ ] - `p1` - **IF** the account has neither a binding nor any observation - `inst-mr-detach-known`
   1. [ ] - `p1` - **RETURN** not-found, nothing appended - `inst-mr-detach-unknown`
4. [ ] - `p1` - Mint a new `person_id` (random UUIDv7) - `inst-mr-detach-mint`
5. [ ] - `p1` - Apply `cpt-ir-algo-manual-resolution-apply-decision` with the new person - `inst-mr-detach-apply`
6. [ ] - `p1` - **RETURN** the new `person_id` - `inst-mr-detach-return`

### Exclude a Non-Person Account

- [ ] `p1` - **ID**: `cpt-ir-flow-manual-resolution-exclude`

**Actor**: `cpt-ir-actor-operator`

**Success Scenarios**:
- The account binds to the reserved excluded person; its activity is attributed to nobody and it leaves the queue

**Error Scenarios**:
- The account is unknown: nothing has observed it and it has no binding

**Steps**:
1. [ ] - `p1` - Operator submits the exclusion - `inst-mr-exclude-submit`
2. [ ] - `p1` - API: POST /v1/resolution/exclude (account) - `inst-mr-exclude-api`
3. [ ] - `p1` - **IF** the account has neither a binding nor any observation - `inst-mr-exclude-known`
   1. [ ] - `p1` - **RETURN** not-found, nothing appended - `inst-mr-exclude-unknown`
4. [ ] - `p1` - Apply `cpt-ir-algo-manual-resolution-apply-decision` with the excluded sentinel - `inst-mr-exclude-apply`
5. [ ] - `p1` - **RETURN** confirmation - `inst-mr-exclude-return`

## 3. Processes / Business Logic (CDSL)

### Apply One Decision

- [ ] `p1` - **ID**: `cpt-ir-algo-manual-resolution-apply-decision`

**Input**: operator person id, target account, target person, reason code, comment

**Output**: applied / no-op, with the appended row count

**Steps**:
1. [ ] - `p1` - DB: SELECT persons (latest `value_type='id'` row for the account: current person and its author) - `inst-mr-apply-current`
2. [ ] - `p1` - **IF** the current binding names the target person **AND** was authored by an operator - `inst-mr-apply-idempotent-check`
   1. [ ] - `p1` - **RETURN** no-op (the identical decision is already recorded) - `inst-mr-apply-noop`
3. [ ] - `p1` - Claim a free observation timestamp for the row per `cpt-ir-algo-manual-resolution-timestamp-slot` - `inst-mr-apply-timestamp`
4. [ ] - `p1` - DB: INSERT persons (binding observation: account, target person, operator as author, reason code) - `inst-mr-apply-append`
5. [ ] - `p1` - Leave the derived caches to the persons-seed: a correction appends to the journal only, and every read path reads the journal - `inst-mr-apply-cache`
6. [ ] - `p1` - DB: INSERT operations (actor, request, comment, outcome) - `inst-mr-apply-journal`
7. [ ] - `p1` - **RETURN** applied - `inst-mr-apply-return`

### Resolve an Addressed Account

- [ ] `p1` - **ID**: `cpt-ir-algo-manual-resolution-address-account`

**Input**: an account reference — either source type + account id, or an observed value (e-mail / username)

> Scope note: the first iteration accepts the direct form only; value addressing (and with it the bulk import of a prepared matching table) is reserved — the per-item skip reasons below are its contract.

**Output**: exactly one account key, or a machine-readable skip reason

**Steps**:
1. [ ] - `p1` - **IF** the reference names source type and account id - `inst-mr-addr-direct`
   1. [ ] - `p1` - **RETURN** that account key (it may not have been observed yet — pre-registration is allowed) - `inst-mr-addr-direct-return`
2. [ ] - `p1` - DB: SELECT identity_inputs (accounts observing the value, folded per account over UPSERT/DELETE); an absent evidence relation reads as no observations, never as a failure - `inst-mr-addr-evidence`
3. [ ] - `p1` - **IF** exactly one active account observes the value - `inst-mr-addr-unique`
   1. [ ] - `p1` - **RETURN** that account key - `inst-mr-addr-unique-return`
4. [ ] - `p1` - **ELSE** - `inst-mr-addr-ambiguous`
   1. [ ] - `p1` - **RETURN** skip reason (`ambiguous_value` or `unknown_value`) — never a guess - `inst-mr-addr-skip`

### Claim an Observation Timestamp Slot

- [ ] `p1` - **ID**: `cpt-ir-algo-manual-resolution-timestamp-slot`

**Input**: the rows to append within one operation

**Output**: a distinct `created_at` per row within the natural observation key

**Steps**:
1. [ ] - `p1` - Take the operation's wall-clock instant as the first candidate - `inst-mr-ts-start`
2. [ ] - `p1` - **FOR EACH** row sharing the natural key (tenant, person, source type, source id, value type) - `inst-mr-ts-loop`
   1. [ ] - `p1` - Advance the candidate by whole microseconds until it is unused - `inst-mr-ts-advance`
3. [ ] - `p1` - **RETURN** the claimed instants - `inst-mr-ts-return`

> Concurrency: allocation is not a lock, so two operations can claim the same instant and the insert silently drops the loser (the key has no account discriminator). A write **MUST** compare what the database appended against what it asked for and, on a short write, ask which of its **exact** observations the journal now holds — author and instant included, because a confirmation writes an operator row over an automatic binding to the same person and "the account points at this person" cannot tell a landed row from a refused one. Only the missing rows are re-stamped and retried; a row that cannot be placed is reported as refused rather than counted as applied.

### Build the Review Queue

- [ ] `p1` - **ID**: `cpt-ir-algo-manual-resolution-review-queue`

**Input**: tenant

**Output**: queue items with candidates, plus resolution-rate shares

**Steps**:
1. [ ] - `p1` - DB: SELECT identity_inputs (observed accounts and their values, folded per account over UPSERT/DELETE so closed accounts drop out) - `inst-mr-queue-evidence`
2. [ ] - `p1` - DB: SELECT persons (current binding and its author per account) - `inst-mr-queue-bindings`
3. [ ] - `p1` - Join evidence to bindings on the account key - `inst-mr-queue-join`
4. [ ] - `p1` - **FOR EACH** identity value claimed by more than one person - `inst-mr-queue-contested`
   1. [ ] - `p1` - **IF** any binding in the group is operator-authored - `inst-mr-queue-settled-check`
      1. [ ] - `p1` - Skip the group (settled by a human) - `inst-mr-queue-settled-skip`
   2. [ ] - `p1` - **ELSE** emit a binding-conflict item with the persons involved - `inst-mr-queue-conflict-item`
5. [ ] - `p1` - **FOR EACH** unbound account whose evidence is contested - `inst-mr-queue-pending`
   1. [ ] - `p1` - Emit a pending item with the candidate persons and the contested values - `inst-mr-queue-pending-item`
6. [ ] - `p1` - **FOR EACH** unbound observed account with no usable identity evidence — an account already bound, including one bound to the excluded person, is not pending anything - `inst-mr-queue-noevidence`
   1. [ ] - `p1` - Emit a no-evidence item (never hidden) - `inst-mr-queue-noevidence-item`
7. [ ] - `p1` - Compute shares: bound / pending / no-evidence / excluded over observed accounts - `inst-mr-queue-rates`
8. [ ] - `p1` - **RETURN** items and shares - `inst-mr-queue-return`

### Resolve Person for Analytics (macro upgrade)

- [ ] `p1` - **ID**: `cpt-ir-algo-manual-resolution-resolver-upgrade`

**Input**: the journal mirror and the evidence, at build time

**Output**: `person_id` per fact, or NULL

**Steps**:
1. [ ] - `p1` - Build the account map: latest `value_type='id'` binding per source-instance-scoped account key - `inst-mr-res-account-map`
2. [ ] - `p1` - Build the value map: each identity value to the accounts observing it, resolved through the account map - `inst-mr-res-value-map`
3. [ ] - `p1` - **IF** a fact carries a source account - `inst-mr-res-account-first`
   1. [ ] - `p1` - **RETURN** the account's bound person - `inst-mr-res-account-return`
4. [ ] - `p1` - **ELSE** look the fact's value up in the value map — built from **every** value an account has carried, not only its current one, so a person who changed address keeps the facts recorded under the old one - `inst-mr-res-fallback`
   1. [ ] - `p1` - **IF** all observing accounts resolve to one person - `inst-mr-res-value-unique`
      1. [ ] - `p1` - **RETURN** that person - `inst-mr-res-value-return`
   2. [ ] - `p1` - **ELSE RETURN** NULL (contested or unknown — excluded, never tie-broken) - `inst-mr-res-null`
5. [ ] - `p1` - Map the excluded sentinel to NULL wherever it appears - `inst-mr-res-sentinel`

## 4. States (CDSL)

### Queue Item Lifecycle

- [ ] `p2` - **ID**: `cpt-ir-state-manual-resolution-queue-item`

**States**: Surfaced, Absent

**Initial State**: Surfaced

**Transitions**:
1. [ ] - `p1` - **FROM** Surfaced **TO** Absent **WHEN** an operator decision removes the item's condition - `inst-mr-state-decided`
2. [ ] - `p1` - **FROM** Surfaced **TO** Absent **WHEN** the account's evidence closes (latest event is a DELETE) - `inst-mr-state-closed`
3. [ ] - `p1` - **FROM** Absent **TO** Surfaced **WHEN** new evidence re-opens the condition - `inst-mr-state-reopened`

## 5. Definitions of Done

### Correction Verbs

- [ ] `p1` - **ID**: `cpt-ir-dod-manual-resolution-verbs`

The system **MUST** expose bind (single and bulk), merge, detach and exclude; each appends binding observations authored by the operator with a machine-readable reason and **MUST NOT** update or delete any existing row. A correction **MUST NOT** rebuild the tenant's derived caches: that work is whole-tenant and superlinear in the tenant's size, so it belongs to the persons-seed's own schedule, not to a request. The journal is what every read path reads, so a correction takes effect the moment it commits.

**Implements**:
- `cpt-ir-flow-manual-resolution-review-and-bind`
- `cpt-ir-flow-manual-resolution-merge`
- `cpt-ir-flow-manual-resolution-detach`
- `cpt-ir-flow-manual-resolution-exclude`

**Touches**:
- API: `POST /v1/resolution/bind`, `POST /v1/resolution/merge`, `POST /v1/resolution/detach`, `POST /v1/resolution/exclude`
- DB: `persons`, `operations`

### Decision-Aware Idempotency

- [ ] `p1` - **ID**: `cpt-ir-dod-manual-resolution-idempotency`

The system **MUST** treat a correction as a no-op only when an identical operator decision is already recorded — the current binding names the same person **and** was authored by an operator. A bind onto an automation-authored binding of the same person **MUST** append the operator's confirmation.

**Implements**:
- `cpt-ir-algo-manual-resolution-apply-decision`

**Touches**:
- DB: `persons`

### Timestamp Uniqueness

- [ ] `p1` - **ID**: `cpt-ir-dod-manual-resolution-timestamps`

The system **MUST** claim a distinct `created_at` per appended row within the natural observation key, so that rows of one operation cannot collide and be dropped by the insert. On a short write it **MUST** identify the rows that did not land — by their full observation identity, not by the account's current person — and retry only those, so recovery cannot duplicate history, and **MUST** report a row it could not place rather than counting it as applied.

**Implements**:
- `cpt-ir-algo-manual-resolution-timestamp-slot`

**Touches**:
- DB: `persons`

### Excluded Sentinel

- [ ] `p1` - **ID**: `cpt-ir-dod-manual-resolution-sentinel`

The system **MUST** bind excluded accounts to the reserved excluded person and **MUST** treat that person as "no person" in every consumer: NULL in the analytics resolver, not served as a person by the read API, ignored by golden-record consumers, hidden from the queue.

**Implements**:
- `cpt-ir-flow-manual-resolution-exclude`

**Touches**:
- DB: `persons`
- Entities: excluded-person sentinel

### Review Queue and Match Rate

- [ ] `p1` - **ID**: `cpt-ir-dod-manual-resolution-queue`

The system **MUST** derive the queue from the evidence fold joined with current bindings, emit pending / binding-conflict / no-evidence items with candidates, suppress divergence explained by an operator-authored binding, and report resolution-rate shares. It **MUST NOT** persist queue state.

**Implements**:
- `cpt-ir-algo-manual-resolution-review-queue`

**Touches**:
- API: `GET /v1/resolution/attention`
- DB: `identity_inputs`, `persons`

### Binding History and Person Accounts

- [ ] `p2` - **ID**: `cpt-ir-dod-manual-resolution-history`

The system **MUST** return, for an account, its current binding and full decision history with authors and reasons; and, for a person, every account and identity value bound to them with the author of each link.

**Implements**:
- `cpt-ir-flow-manual-resolution-review-and-bind`

**Touches**:
- API: `GET /v1/resolution/accounts/{source}/{source_id}/{account_id}`, `GET /v1/resolution/persons/{person_id}/accounts`
- DB: `persons`, `identity_inputs`

### Operator Authorization

- [ ] `p1` - **ID**: `cpt-ir-dod-manual-resolution-authz`

The system **MUST** require an operator grant for every write verb, enforced through the service's existing role grants, and **MUST** record the acting person on every appended row and journal entry. Every read and write — queue, account history, person accounts, binding lookups and appended rows — **MUST** be scoped to the tenant of the verified caller, taken from the request context and never from caller-supplied input.

**Implements**:
- `cpt-ir-flow-manual-resolution-review-and-bind`

**Touches**:
- DB: `roles`, `person_roles`, `persons`, `operations`

### Analytics Resolver Upgrade

- [ ] `p1` - **ID**: `cpt-ir-dod-manual-resolution-resolver`

The system **MUST** resolve `person_id` account-first on the source-instance-scoped account key, fall back to an account-derived value map — covering every value an account has ever carried — for facts without an account, resolve contested values to NULL, and map the excluded sentinel to NULL. Where the connector evidence has never been built, resolution **MUST** degrade to resolving nothing rather than failing the build; it **MUST NOT** create or pre-create relations the transformation layer owns.

**Implements**:
- `cpt-ir-algo-manual-resolution-resolver-upgrade`

**Touches**:
- Entities: analytics resolution macro, gold observation models

## 6. Acceptance Criteria

- [ ] An operator decision survives a subsequent seed run byte-for-byte, and the seed reuses it
- [ ] Binding an account to the person it already has, when that binding was written by automation, appends an operator confirmation and clears the queue item
- [ ] Re-submitting an identical operator decision (including a re-uploaded bulk file) changes nothing and is reported as a no-op
- [ ] A merge rebinds every account of the absorbed person; a detach works on an account with no prior merge record
- [ ] A correction followed by its counter-correction restores the effective bindings
- [ ] Bulk rows addressed by a value that resolves to zero or several accounts are skipped with a machine-readable reason and remain in the queue (with value addressing; the direct form ships first)
- [ ] Two accounts of one source bound to one person in one operation both persist (distinct timestamps); a short write retries only the rows that did not land and never duplicates one that did
- [ ] Detach and exclude reject an account nothing has observed and that holds no binding — including before the evidence relation has ever been built, where the answer is still not-found
- [ ] A bulk call naming one account twice is rejected, and per-item outcomes are reported by position
- [ ] A person who changed e-mail keeps their history: facts recorded under the previous address still resolve
- [ ] The queue surfaces pending, binding-conflict and no-evidence items, suppresses operator-settled divergence, hides excluded accounts, and reports resolution-rate shares
- [ ] Accounts whose latest evidence event is a closure do not appear in the queue
- [ ] After a correction, the next gold build attributes the affected accounts' full history to the corrected person; contested values resolve to NULL rather than a winner
- [ ] Excluded accounts resolve to NULL in analytics and are not served as persons by the read API
- [ ] Write verbs are rejected without an operator grant, and every appended row names the acting person
