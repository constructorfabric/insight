---
id: cpt-ir-adr-operator-decisions-as-observations
status: proposed
date: 2026-08-05
decision-makers: mozhaev-dev
---

# ADR-0003 — Operator identity corrections as append-only `persons` observations (no separate decision store in v1)


<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Operator decisions as ordinary `persons` binding observations](#operator-decisions-as-ordinary-persons-binding-observations)
  - [Snapshot-based merge/split over ClickHouse `aliases` + `merge_audits`](#snapshot-based-mergesplit-over-clickhouse-aliases--merge_audits)
  - [Dedicated `identity_decisions` journal (must-link / cannot-link decision store)](#dedicated-identity_decisions-journal-must-link--cannot-link-decision-store)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-ir-adr-operator-decisions-as-observations`
## Context and Problem Statement

Automatic e-mail-based resolution can err in both directions: under-merge (one human split across two persons when their e-mails differ, e.g. `alice@example.com` vs `alice@personal.example`) and over-merge (two humans grouped through a shared value such as `team@example.com`). The product offers no supported way to change a binding once automation has written it, so neither error is correctable through any product interface; additionally, the seed's divergent-group collapse can silently re-derive an existing binding (see the seed-hardening enabler below).

We are adding operator correction verbs (bind, merge, detach, exclude — see constructorfabric/insight#2180 for the reviewed design). The question this ADR answers: **where do operator decisions live, and how are they enforced so that they survive every re-run of the automatic pipeline?**

## Decision Drivers

* Operator decisions must survive seed re-runs and future automation **by construction**, not by special-casing (epic #1873: "the override store is the source of truth; the auto-resolver only proposes").
* Never merge silently: contested cases must surface to a human; no automation path may collapse two existing persons (BR-5 of #1602).
* Full audit: who decided what, when, and why — for every correction.
* Corrections must reach analytics through the existing pipeline (`persons` -> ClickHouse mirror -> dbt `resolve_person_id` macro -> gold), with changes confined to that pipeline's designated extension points.
* Smallest implementable slice for the 2026-08-25 release window of issue #2180.
* A future auto-matcher (confidence-scored proposals, #796/#1790) must be layerable on top without rework.

## Considered Options

* Operator decisions as ordinary `persons` binding observations
* Snapshot-based merge/split over ClickHouse `aliases` + `merge_audits`
* Dedicated `identity_decisions` journal (must-link / cannot-link decision store)

## Decision Outcome

Chosen option: "Operator decisions as ordinary `persons` binding observations", because decisions written in the same currency the seed reads need no parallel override store, require no schema migration for this iteration (given enabler 4 below), and ship within the release window while keeping all richer machinery layerable later. Survival through re-seeding rests on one seed invariant — an account's existing binding is reused, never re-derived — which already holds for consistently-bound groups and is extended to divergent groups by the seed hardening below.

The decision in one sentence: **an operator correction is a new binding observation appended to `persons`, written in the same currency the automation already respects.**

Recorded semantics:

* Each operator verb appends binding observations (`value_type='id'`) with `author_person_id` = the operator's real UUID (never the seed sentinel) and a machine-readable `reason` code: `operator-bind`, `operator-merge`, `operator-detach`, `operator-exclude`. Free-text commentary and request payloads are captured in the existing `operations` journal, which was designed for reuse by new admin operations.
* Current state remains "latest binding per account wins"; history is never edited or deleted.
* A correction writes binding observations only. The journal does not encode which identity value belongs to which account — the account's own `value_type='id'` row carries the account id in `value_id`, but attribute observations (e-mail, username) reference only the source instance; that linkage lives in the `identity_inputs` evidence (`source_account_id` on every row), which the resolver and the value-addressed bulk bind consume. E-mail resolution therefore follows corrections indirectly, through the current bindings of the accounts observing each e-mail (enabler 1).
* "Confirm" (an account pending review is genuinely a separate person) is bind-to-self: the same person, re-asserted under an operator author.
* "Exclude" (bots, service accounts) binds the account to the reserved excluded-person sentinel — a fixed, unmintable UUID defined normatively in DESIGN par. 4.3, treated by every consumer (resolve macro, read API, person domain, review queue) as "no person".
* Undo is a counter-action (a newer binding), never a destructive revert; the journal retains the mistake, the fix, and both authors.
* The review queue is derived from two sources joined on the account key — the `identity_inputs` evidence (every observed account, including e-mail-less ones; folded per account over UPSERT/DELETE events so closed accounts drop out) and the current bindings in `persons` — with no persisted status columns. A divergence explained by an operator-authored binding is a resolved state, not a conflict — this classification-by-author is what keeps settled decisions out of the queue.

Required enablers shipping with the feature (this decision is incomplete without them):

1. **dbt resolver upgrade** — the v1 `resolve_person_id` macro resolves by e-mail only, so `value_type='id'` corrections would never reach gold. The macro gains an account-first map (latest `id` binding per source account) and an **account-derived e-mail fallback** for facts that carry no account (e.g. git commits): each e-mail maps to the accounts observing it in `identity_inputs` and resolves through their current bindings — all on one person → that person; several persons or none → NULL (contested evidence is excluded, not tie-broken). Known limit: facts keyed by an e-mail never observed on any account stay NULL.
2. **Seed hardening** — the current seed collapses a divergent e-mail group onto its first binding, which could silently override an operator row. Per-account bindings must win over group collapse; the bindings loader must return the binding author so divergence can be classified (operator-authored = resolved state; all-seed = surfaced); a contested e-mail stops auto-linking new accounts.
3. **Decision-aware API-level idempotency** — the journal's natural key includes `created_at` (service migration 004), so re-applying a correction would append a new history row rather than collide. The API therefore treats an item as a no-op **only when an identical operator decision is already recorded** (same target person and operator-authored effective binding). A bind-to-self over an automation-authored binding is not a no-op — it is the "confirm" act and appends the operator row.
4. **Unique per-row observation timestamps** — the natural key carries no account discriminator, so two `id` observations for two accounts of one source, bound to one person at the same `created_at`, would collide and `INSERT IGNORE` would drop one. The correction path allocates strictly increasing `DATETIME(6)` timestamps per affected row within an operation (bulk included); the seed carries the same obligation for accounts of one source resolving to one person at the same `_synced_at`. Extending the key with an account discriminator is a candidate follow-up migration.

### Consequences

* Good, because durability needs no parallel store or replay machinery — one existing seed invariant (bindings are reused, never re-derived), extended by a small hardening to divergent groups, covers it.
* Good, because the blast radius is small and confined to designated extension points: no new tables; new endpoints in the identity-resolution service; the resolver upgrade lands inside the one macro that owns resolution semantics; the seed hardening is a focused change in `resolve_assignments` and its bindings loader.
* Good, because the journal doubles as an audit trail and as future training data for the auto-matcher (every operator decision is a labeled example).
* Good, because the divergent-group collapse that could silently rewrite bindings is eliminated by the same hardening that protects operator rows.
* Bad, because there is no first-class negative assertion ("never merge these two"): acceptable while the only automation never merges existing persons and a single operator is assumed, but it must be revisited (see below).
* Bad, because n-way splits are a sequence of detaches (not atomic), reverts are manual counter-actions, concurrent operators are last-write-wins, and there is no ignore/defer for queue items (a snooze needs persisted queue state; it returns with the proposal store).
* Neutral, because GDPR erasure does not conflict with append-only: the rule governs identity decisions, while lawful erasure of stored identity values is an explicit administrative operation outside the decision journal (future purge flow), itself recorded in the `operations` journal.

Revisit triggers — any of these reopens the decision in favour of a dedicated decision journal layered on top (operator rows in `persons` remain valid as materialisations, so no rework is lost):

1. An auto-matcher that produces merge proposals ships.
2. More than one concurrent operator per tenant.
3. Recurring manual re-decisions on the same account pairs (a signal that "these are different people" needs to be stored as a rule).
4. Need for value blocklists (shared mailboxes, bot addresses) as first-class objects.

### Confirmation

* Design review of constructorfabric/insight#2180 (scenarios S1-S10 cover each verb, seed re-run survival, conflict classification, and retroactive metric re-attribution).
* Seed re-run test: an operator-authored binding is untouched by a subsequent seed run.
* Conflict-classification test: an e-mail group whose binding divergence includes an operator-authored binding produces no conflict item and no crash.
* End-to-end test: activity keyed by a corrected account re-attributes to the new person on the next gold build.

## Pros and Cons of the Options

### Operator decisions as ordinary `persons` binding observations

Operator verbs append binding rows to the existing append-only `persons` log; audit metadata rides on the existing `author_person_id` / `reason` columns plus the `operations` journal.

* Good, because survival through re-seeding is structural (the binding-reuse invariant plus its divergent-group hardening), not a bolted-on exception list.
* Good, because no new tables or migrations are needed for this iteration, and the downstream change is confined to the one macro that owns resolution semantics; implementable within the release window.
* Good, because provenance (human vs automation) is a load-bearing field already present on every row.
* Neutral, because negative knowledge is captured only implicitly (a detach's history implies "this account is not that person") — recoverable later as training/backfill material.
* Bad, because rich decision semantics (cannot-link, atomic partition, retract-with-reapply, resolution epochs) have no first-class home yet.

### Snapshot-based merge/split over ClickHouse `aliases` + `merge_audits`

The late-phase plan of DESIGN v2.0: reassign `aliases` rows between persons, snapshot before/after into `merge_audits`, split = restore a snapshot.

* Good, because it was already written down in the spec and covers GDPR archival flows.
* Bad, because none of the required tables (`aliases` as resolution store, `unmapped`, `conflicts`, `merge_audits`) is actually used by the implemented resolution path — the plan targets an architecture that was not built.
* Bad, because the mechanics are unsound on ClickHouse: `ALTER TABLE UPDATE` is an asynchronous mutation (the post-merge snapshot can read pre-mutation state), and the proposed ReplacingMergeTree keys (with unique `id` in ORDER BY, mutable `status` in ORDER BY) prevent the dedup/collapse the design relies on.
* Bad, because split exists only as a rollback of a recorded merge — it cannot separate persons grouped by the seed itself (e.g. through a shared mailbox), a case the split verb must handle without any prior merge record.
* Bad, because a status-column queue table drifts from reality (resolved items linger).

### Dedicated `identity_decisions` journal (must-link / cannot-link decision store)

A separate append-only decision table (+ subjects), applied as constraints on every resolution pass; positive decisions materialise as `persons` bindings.

* Good, because negative assertions, atomic n-way partition, retract-with-reapply, conflict responses citing decisions, and resolution epochs all get a first-class home — this matches industry-converged practice (pairwise judgement journals, steward decision tables).
* Good, because it is the natural v2 once an auto-matcher or multi-operator concurrency arrives.
* Bad, because it requires new tables, a validation/conflict engine, and fold semantics now — oversized for a single-operator MVP whose only automation never merges existing persons.
* Bad, because two journals create a source-of-truth split that must be actively reconciled (decisions win; bindings are materialisations) — complexity with no current payoff.

This option is **deferred, not rejected**; the revisit triggers above name the conditions.

## More Information

Industry prior art converges on the chosen shape: decisions stored as data that the matcher re-consumes on every run (OpenSanctions/nomenklatura pairwise judgements; Senzing trusted identifiers encoded in records; Semarchy steward decisions as table rows; SortingHat locked profiles), and merge implemented as reversible linking rather than physical collapse (documented irreversible-merge pain in Segment/Mixpanel/PostHog). The reviewed design with scenarios and API shapes lives in constructorfabric/insight#2180; the umbrella vision is #1873; related edge-case requirements: #1767, #1776.

When the matcher iteration lands, proposal `confidence` and `evidence` receive first-class storage (reserved column names); the journal deliberately carries no dead columns for them now — additive MariaDB migrations are cheap at that point, and operator provenance (author, reason, `operations` payload) already accumulates the training signal in the meantime. This is a deliberate, recorded deviation from the review suggestion to reserve physical columns in the MVP schema.

This ADR implements the operator-flow consequence promised by [ADR-0002](0002-stable-person-id-via-persons-observations.md) ("Operator-driven flows (future PR) will create new persons rows ... with author_person_id = the operator's person_id and a descriptive reason").

## Traceability

- **PRD**: [PRD.md](../PRD.md)
- **DESIGN**: [DESIGN.md](../DESIGN.md)

This decision directly addresses the following requirements or design elements:

* `cpt-ir-fr-merge-v2` — merge of two persons is an appended reassignment of account bindings, recorded with actor and reason.
* `cpt-ir-fr-split-v2` — detach/split works on any account regardless of prior merge history; no snapshot restore required.
* `cpt-ir-fr-merge-audit-v2` — the journal itself is the audit trail (author, reason, timestamp on every row; request payloads in the `operations` journal).
* `cpt-ir-fr-idempotent-mutations-v2` — corrections are idempotent through the decision-aware API check (enabler 3): re-applying an already-recorded operator decision is a reported no-op.
* `cpt-ir-actor-operator` — the operator becomes a first-class author in the `persons` journal.
