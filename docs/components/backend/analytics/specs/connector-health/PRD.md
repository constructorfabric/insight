# PRD — Connector Health


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
  - [5.1 Recording Syncs](#51-recording-syncs)
  - [5.2 Presenting State](#52-presenting-state)
  - [5.3 Degradation](#53-degradation)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
  - [6.1 NFR Inclusions](#61-nfr-inclusions)
  - [6.2 NFR Exclusions](#62-nfr-exclusions)
- [7. Public Library Interfaces](#7-public-library-interfaces)
  - [7.1 Public API Surface](#71-public-api-surface)
  - [7.2 External Integration Contracts](#72-external-integration-contracts)
- [8. Use Cases](#8-use-cases)
  - [UC-1 — A failing connector is triaged from the product](#uc-1--a-failing-connector-is-triaged-from-the-product)
  - [UC-2 — A silent stop is noticed](#uc-2--a-silent-stop-is-noticed)
  - [UC-3 — A fresh install is read correctly](#uc-3--a-fresh-install-is-read-correctly)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Dependencies](#10-dependencies)
- [11. Assumptions](#11-assumptions)
- [12. Open Decisions](#12-open-decisions)
- [13. Risks](#13-risks)

<!-- /toc -->

Scoped addition to the [Analytics service](../../DESIGN.md) and the reconcile loop: what an
instance operator must be able to learn about the state of every connector, and what the
product may and may not claim about it. Realised by [DESIGN.md](DESIGN.md). Supersedes the
current Data health pane and the sketch in
[PR #2713](https://github.com/constructorfabric/insight/pull/2713), which is not merged.

## 1. Overview

### 1.1 Purpose

Give the instance operator one admin surface that answers, per connector: is it configured,
when did its last sync run, and how did that sync end — and, on expanding the row, how often
it has been failing. The surface reports recorded facts about syncs; it never infers a verdict
it cannot back.

### 1.2 Background / Problem Statement

The product today has no surface that reports connector state. The pane named *Data health*
counts schema-check statuses of metric definitions — a fact about the metric catalogue, not
about whether data arrives — and three of its four counters overstate. Custom metrics carry
no schema status by design, so they inflate both *unchecked* and *no data yet*; disabled
definitions count as health. A schema-broken definition that has never produced a row is
counted twice, under two different counters.

The consequence is operational: a connector that quietly stops delivering looks identical to
one that was never configured, and the customer notices empty dashboards before the operator
notices the connector. Related backlog items describe the same pain from the alerting side
([#744](https://github.com/constructorfabric/insight/issues/744),
[#723](https://github.com/constructorfabric/insight/issues/723)); this PRD covers the
observation surface, not notifications.

Two failure classes are invisible today even to a diligent operator reading dashboards:

| Failure | Why it is invisible |
|---|---|
| Sync fails repeatedly | old data still renders; nothing states that ingestion is failing |
| A connector is removed or never configured | its schema still exists, so absence looks like emptiness |

### 1.3 Goals (Business Outcomes)

- The operator learns that a connector's syncs are failing from the product, the same day
  they start failing — not from a user reporting an empty dashboard.
- Every sync the mover ran leaves a durable, queryable record that outlives the mover's own
  job retention.
- The page distinguishes *failing*, *running*, *never ran*, and *no longer configured* —
  states that are conflated today.
- No verdict is presented that the recorded facts cannot back; absence of knowledge renders
  as *unknown*, never as *healthy*.

### 1.4 Glossary

| Term | Meaning |
|---|---|
| **Connector** | one ingestion source type configured on the install; its raw data lands in a per-connector bronze schema |
| **Sync** | one execution of the data mover for one connector's connection |
| **Sync ledger** | the append-only record of sync outcomes this PRD introduces |
| **Sweep** | the periodic reconciliation that copies sync outcomes from the mover's job history into the ledger, and records which connectors the install has configured |
| **Configured connector** | a connector the install has supplied credentials for — which is the criterion the controller itself acts on — and which is present in its newest COMPLETE snapshot; absence from that snapshot means no longer configured. A snapshot still being written is not one. Every connector the build ships is a wider set and is not this one |
| **Reported records** | the record count the mover states for a sync. It describes what the mover sent, not what storage kept |

## 2. Actors

### 2.1 Human Actors

#### Instance operator

**ID**: `cpt-insightspec-connhealth-actor-operator`

**Role**: administers one Insight install; configures connectors and is accountable for data
arriving.
**Needs**: to see which connectors are configured, when each last synced, how that sync
ended, and whether a quiet connector is failing, stopped, or was never configured. The
operator is the only human actor permitted to read this surface.

### 2.2 System Actors

#### Reconcile loop

**ID**: `cpt-insightspec-connhealth-actor-reconcile`

**Role**: the periodic controller that provisions connections from connector configuration.
Sole writer of the sync ledger: each tick it sweeps the mover's job history and records sync
outcomes, plus a snapshot of the connector set it manages.

#### Data mover

**ID**: `cpt-insightspec-connhealth-actor-mover`

**Role**: executes syncs and keeps a bounded job history with per-job outcome, timestamps and
a reported record count. Source the sweep reads; never read on the page's request path.

#### Warehouse

**ID**: `cpt-insightspec-connhealth-actor-warehouse`

**Role**: stores the sync ledger and serves it to the read surface.

#### Analytics service

**ID**: `cpt-insightspec-connhealth-actor-analytics`

**Role**: serves the read surface to the portal from the ledger alone.

#### Portal

**ID**: `cpt-insightspec-connhealth-actor-portal`

**Role**: renders the admin page: one summary row per connector, expandable to that
connector's recent syncs.

## 3. Operational Concept & Environment

Facts flow one way. On a fixed cadence the reconcile loop reads the mover's job history and
records each sync's outcome into the sync ledger, alongside a snapshot of the connectors it
manages. At read time the analytics service answers from the ledger alone — no call leaves
the warehouse on the request path, and nothing the page shows depends on the mover being
reachable.

### 3.1 Module-Specific Environment Constraints

- **Bronze schemas are not tenant-partitioned**, so this read is instance-wide and gated on
  the operator role rather than scoped by tenant.
- **Bronze schemas exist for every connector the build knows**, configured or not; storage
  presence alone cannot distinguish *never configured* from *configured and broken*. The
  recorded configured set provides that distinction.
- **Sync schedules live in the cluster's workflow layer**, not in the mover; the mover cannot
  say when the next sync is due, only what its past jobs did.
- **Freshness thresholds are declared per connector in transform-layer source definitions
  and have no runtime-readable form.** Until they do, the surface reports facts without
  fresh/stale verdicts.
- **Development compose stands run no mover.** The surface is not supported there beyond not
  breaking: it renders its degraded state (see FR-10).

## 4. Scope

### 4.1 In Scope

- The sync ledger: an append-only record of sync outcomes, written by the sweep.
- Sweep coverage of every sync in the mover's job history, and one-time backfill of the
  ledger from that history.
- Recording of the currently configured connector set, so *never configured* is
  distinguishable from *configured and never ran*.
- The read surface: two admin endpoints and one portal page — per-connector summary and
  per-connector sync history.
- Replacing the current Data health pane with this page.

### 4.2 Out of Scope

- **Whether a sync's data reached storage.** The page reports what the mover states it moved
  and does not verify it against storage. Pairing a self-report with an independent count
  requires measuring at sync time inside the ingestion pipeline, which this iteration does
  not do.
- **Transform outcome.** Whether downstream layers rebuilt after a sync is not recorded, so
  "sync succeeded, transform failed" is not a state this surface can show.
- **How a sync was started.** The mover's job history does not say who triggered a job, and
  nothing here infers it.
- **Per-stream volume** — stream counts, stored rows and size are not recorded.
- **Alerting and notifications** — this PRD makes state observable;
  [#723](https://github.com/constructorfabric/insight/issues/723) owns pushing it.
- **Expectation and alerting on the connector set** — this surface records and shows what
  *is* configured (FR-4); deciding what *should* be configured for a given customer, and
  alerting on the gap, belongs to
  [#744](https://github.com/constructorfabric/insight/issues/744).
- **Freshness verdicts** (fresh / warn / stale) — deferred until declared thresholds have a
  runtime source.
- **Compose-stand support** beyond graceful degradation.
- **Fleet-level views across installs**
  ([#1816](https://github.com/constructorfabric/insight/issues/1816)).

## 5. Functional Requirements

### 5.1 Recording Syncs

#### FR-1 — Every sync in the mover's history is recorded

- [ ] `p1` - **ID**: `cpt-insightspec-connhealth-fr-all-syncs-recorded`

Sync outcome, start time, duration and the mover's reported record count are recorded for
every sync job in the mover's history, whatever started it. Each record carries the mover's
job identity, so re-reading the same history changes nothing. In steady state a sync appears
in the ledger no later than one sweep interval after the mover reports it; while a backfill is
still catching up it appears within the passes that backfill takes (FR-3).

A job the mover has not finished is recorded with the outcome it has, and re-recorded on a
later tick once it ends: a provisional state never closes a job.

#### FR-2 — History is bounded and survives the mover

- [ ] `p2` - **ID**: `cpt-insightspec-connhealth-fr-bounded-history`

Ledger rows are retained for six months — long enough to answer "when did this break" and
"how often does it fail". Retained history outlives connection deletion in the mover, where a
removed connection takes its job history with it.

#### FR-3 — First sweep backfills retained history

- [ ] `p2` - **ID**: `cpt-insightspec-connhealth-fr-backfill`

On an install whose ledger is empty, the first sweep ingests the mover's whole retained job
history rather than only what happened since, so the page has depth on day one. A backfill
that cannot finish in one pass leaves no gap behind it: whatever it did not reach is still
picked up by a later one.

#### FR-4 — The configured set is recorded

- [ ] `p2` - **ID**: `cpt-insightspec-connhealth-fr-configured-set`

Each tick records which connectors the controller manages, as one complete snapshot. A
connector present in the newest complete snapshot is configured; one absent from it but
holding sync history is no longer configured. A snapshot still being written is never read as
a complete one. A tick that cannot read its inputs records nothing rather than recording an
empty set, because an empty set is indistinguishable from "everything was removed".

### 5.2 Presenting State

#### FR-5 — One summary row per connector

- [ ] `p1` - **ID**: `cpt-insightspec-connhealth-fr-summary-row`

Each connector occupies one row carrying: its name, whether it is configured now, when its
last sync started, how long that sync took, how it ended, and the record count the mover
reported for it. Values the ledger does not hold render as unknown.

#### FR-6 — A row expands to its recent syncs

- [ ] `p2` - **ID**: `cpt-insightspec-connhealth-fr-sync-history`

Expanding a row lists that connector's recent syncs newest first — each with its start time,
duration, outcome and reported record count — so a one-off failure is distinguishable from a
repeating one, and a shrinking record count is visible. The list is a bounded window and the
page says so; it is not the full retained history.

#### FR-7 — Facts, not verdicts

- [ ] `p1` - **ID**: `cpt-insightspec-connhealth-fr-facts-not-verdicts`

The surface asserts only what the ledger records. No fresh/stale verdict appears anywhere. A
successful sync is reported as a successful sync, never as delivery: the ledger holds the
mover's own account and nothing that corroborates it. Where a fact is missing the page says
unknown; it never substitutes zero, and never lets an unreadable value read as healthy.

#### FR-8 — Attention orders the page

- [ ] `p2` - **ID**: `cpt-insightspec-connhealth-fr-attention-order`

Rows sort by what needs acting on: failing syncs first, then states the page cannot read,
then everything quiet, with never-ran and never-configured connectors last. Within a band the
most recent activity comes first.

#### FR-9 — Operator-only, instance-wide

- [ ] `p1` - **ID**: `cpt-insightspec-connhealth-fr-operator-only`

The surface is available only to the instance operator role and reports the whole install.
A caller without that role is refused, and the refusal names the surface it was refused.

### 5.3 Degradation

#### FR-10 — The page works with an empty ledger

- [ ] `p2` - **ID**: `cpt-insightspec-connhealth-fr-empty-ledger`

Before the first controller cadence, or on stands where nothing records, the page states
plainly that nothing has been read from the mover yet. It does not error and does not imply
health.

#### FR-11 — The read path depends on nothing external

- [ ] `p1` - **ID**: `cpt-insightspec-connhealth-fr-no-external-reads`

Serving the page requires only the recorded facts. No mover, cluster API, or other external
call is made on the request path, and the reader holds no access to raw connector data in any
form; unavailability upstream cannot degrade the page beyond the staleness of the last
recorded facts.

#### FR-12 — A stopped recorder is visible

- [ ] `p1` - **ID**: `cpt-insightspec-connhealth-fr-stale-recording`

Recording failures are swallowed so they cannot disturb reconciliation (NFR-2), which means a
recorder that stops leaves the page showing its last picture indefinitely. So the page states
when the mover was last read, dating everything below it; and when that read is far older
than the interval between the reads before it, the page says recording appears to have
stopped and the connector states shown may no longer be current.

Both the age and the interval are recorded facts, so the page asserts nothing about a
schedule it cannot see. Where too few reads are recorded to establish an interval, the page
shows the age and says the cadence is unmeasured — it does not conclude that recording
stopped, because with no cadence recorded there is nothing for the age to be late against.

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

#### NFR-1 — Reading is interactive

- [ ] `p2` - **ID**: `cpt-insightspec-connhealth-nfr-interactive-read`

Opening the page returns within one second at p95, with the ledger holding a full retention
window of history, because the read path touches only recorded facts — never raw connector
data, and never a measurement taken while the operator waits.

#### NFR-2 — Recording never breaks reconciliation

- [ ] `p1` - **ID**: `cpt-insightspec-connhealth-nfr-recording-harmless`

A failure anywhere in the sweep must not disturb connector reconciliation around it: no tick
aborts because observability broke. Observability is subordinate to the thing observed.

#### NFR-3 — The sweep is idempotent

- [ ] `p2` - **ID**: `cpt-insightspec-connhealth-nfr-idempotent-sweep`

Re-sweeping the same job history — after a crash, a restart, or overlap — does not change
what the page reports. Duplicate discovery is harmless.

#### NFR-4 — Storage is bounded

- [ ] `p3` - **ID**: `cpt-insightspec-connhealth-nfr-bounded-storage`

The ledger grows with syncs and with controller ticks, and is bounded by retention; it stays
a small service table and never enters customer-facing exports or metric relations.

### 6.2 NFR Exclusions

- **No real-time guarantee** — a sync becomes visible within one sweep interval, not
  instantly; an in-flight sync may be reported as running with sweep-interval staleness.
- **No alerting SLA** — see Out of Scope.
- **No completeness guarantee for pre-ledger history** — backfill reaches only as far as the
  mover's retained job history.

## 7. Public Library Interfaces

### 7.1 Public API Surface

#### Connector health read surface

**ID**: `cpt-insightspec-connhealth-interface-read-surface`

One operator-gated read surface on the analytics service returning the per-connector summary
and the recent-sync history described in §5.2. Contract, shape, and transport are specified
in [DESIGN.md](DESIGN.md).

### 7.2 External Integration Contracts

#### Mover job history

**ID**: `cpt-insightspec-connhealth-contract-mover-history`

The sweep consumes the mover's job listing: per job, its identity, connection, outcome,
creation and start timestamps, duration, and reported record count. This is a background
integration owned by the reconcile loop; the read surface never depends on it.

## 8. Use Cases

### UC-1 — A failing connector is triaged from the product

- [ ] `p1` - **ID**: `cpt-insightspec-connhealth-usecase-broken-sync`

A connector's scheduled sync fails. The operator opens the page, sees the connector sorted to
the top with its last sync failed, and expands the row to the recent syncs — enough to tell a
one-off from a repeating failure, and to see whether the record count had been shrinking
before it stopped. The window is bounded and the page says so; dating the first failure of a
long series is out of scope for this surface.

### UC-2 — A silent stop is noticed

- [ ] `p1` - **ID**: `cpt-insightspec-connhealth-usecase-silent-stop`

A connector's schedule disappears (a suspended or deleted workflow). No new syncs are
recorded. The page shows how long ago the connector's last sync started, and the expansion
lists the start times of the recent ones. The operator reads the gap from those timestamps;
the product computes no cadence and claims no intended schedule.

### UC-3 — A fresh install is read correctly

- [ ] `p2` - **ID**: `cpt-insightspec-connhealth-usecase-fresh-install`

On a new install the operator configures a subset of connectors. The page lists configured
connectors with their first recorded syncs, and sorts a connector that has dropped out of
configuration — one holding sync history but absent from the newest snapshot — to the bottom
rather than reporting it as a failure (FR-4). Before the first controller cadence completes,
the page says nothing has been read from the mover yet rather than implying anything.

A connector that was never configured and has never synced is not listed at all. FR-11 gives
the read surface one relation and nothing else, and such a connector has left no record in
it; listing it would mean reading storage the reader deliberately holds no access to.

## 9. Acceptance Criteria

- [ ] Every job in the mover's history appears in the ledger within one sweep interval, with
      outcome, timestamps, duration and reported record count (FR-1).
- [ ] A job still running is recorded provisionally and re-recorded with its outcome once it
      ends (FR-1).
- [ ] A connector present in the snapshot with no syncs renders as configured and never ran;
      removed from configuration, it stops rendering configured within one controller cadence;
      one that has left no record at all is not listed, because the reader holds one relation
      and nothing else (FR-4, FR-11).
- [ ] A tick whose inputs are unreadable records nothing rather than recording an empty
      configured set (FR-4).
- [ ] Deleting a connection in the mover does not remove already-recorded history (FR-2).
- [ ] On a ledger-less install the first sweep populates history from the mover's retained
      jobs (FR-3).
- [ ] The page renders one row per connector with the fields of FR-5, expandable per FR-6.
- [ ] No fresh/stale verdict appears anywhere, no successful sync is described as delivery,
      and unknown states render as unknown (FR-7).
- [ ] A non-operator caller is refused; the response is instance-wide for the operator
      (FR-9).
- [ ] With the mover unreachable, the page still serves the last recorded facts and the
      reader holds no access to raw connector data (FR-11).
- [ ] The page dates itself by when the mover was last read; with recording stopped long
      enough to stand out against the preceding reads, it says so rather than presenting the
      last picture as current (FR-12).
- [ ] Rows sort worst-first: failing, then unreadable, then quiet, with never-ran and
      no-longer-configured last, and most recent activity first inside a band (FR-8).
- [ ] Before any tick has sealed, the page states that nothing has been read rather than
      rendering an empty install as healthy (FR-10).
- [ ] The summary answers within its budget with a full retention window recorded (NFR-1).
- [ ] Ledger growth is bounded by retention, and no connector's history can grow it without
      bound (NFR-4).
- [ ] A sweep failure does not abort connector reconciliation (NFR-2); a repeated sweep does
      not change reported state (NFR-3).

## 10. Dependencies

- The reconcile loop, which already authenticates to the mover and runs on a periodic tick;
  the sweep extends it.
- The warehouse, which stores the ledger.
- Replacement target: the Manage-zone Data health pane in the portal.

## 11. Assumptions

- The mover's job listing is the complete account of syncs for a connection: a sync that ran
  appears there whatever started it.
- The mover retains job history long enough for a useful backfill window; retention shorter
  than ledger retention limits backfill, not steady-state operation. Its listing is paged, so
  the window is the mover's retention rather than one page of it.
- A connector name carries no underscore, so the name the read surface parses out of a URL
  path is the name the descriptor declares. Enforced by the connector wiring guard rather
  than assumed: a name that breaks it would leave that connector's history unreachable
  behind a row the page had already drawn.

## 12. Open Decisions

- ~~Ledger retention length~~ — settled at six months.
- ~~Per-stream moved-record counters from the mover's job detail~~ — deferred: the sweep
  consumes the public listing only, so nothing depends on an API that is not
  contract-stable across mover upgrades.
- ~~Naming of the replacement pane~~ — settled as Connector health.
- Whether the sync history needs paging beyond its bounded window. UC-1 is served by the
  window; dating the first failure of a long series is not, and would need a cursor on the
  read surface. **Owner**: this spec's author, to settle after the page has been used for
  triage — not before this iteration ships.

## 13. Risks

- **The sweep reads the mover's API across versions.** Mitigation: consume only its stable
  public listing for outcomes; treat any richer detail as optional enrichment that may
  degrade without degrading the page (FR-11 keeps the read path independent).
- **The mover's account is uncorroborated.** A sync it calls successful may have delivered
  nothing to storage, and this surface cannot tell. Mitigation is honesty rather than
  detection: the page reports the count as reported, and no state on it says "delivering".
  Closing the gap needs measurement at sync time and is out of scope here.
- **A tick can record a partial picture.** Mitigation: a tick's snapshot becomes readable
  only once it is complete, so a half-written tick is never read as a whole one. How
  completeness is established is DESIGN's.
