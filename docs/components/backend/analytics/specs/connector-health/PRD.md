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
  - [5.1 Recording Runs](#51-recording-runs)
  - [5.2 Presenting State](#52-presenting-state)
  - [5.3 Degradation](#53-degradation)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
  - [6.1 NFR Inclusions](#61-nfr-inclusions)
  - [6.2 NFR Exclusions](#62-nfr-exclusions)
- [7. Public Library Interfaces](#7-public-library-interfaces)
  - [7.1 Public API Surface](#71-public-api-surface)
  - [7.2 External Integration Contracts](#72-external-integration-contracts)
- [8. Use Cases](#8-use-cases)
  - [UC-1 — A broken sync is triaged from the product](#uc-1--a-broken-sync-is-triaged-from-the-product)
  - [UC-2 — A silent stop is noticed](#uc-2--a-silent-stop-is-noticed)
  - [UC-3 — A manual sync is accounted for](#uc-3--a-manual-sync-is-accounted-for)
  - [UC-4 — A sync that delivered nothing is distinguished from a healthy one](#uc-4--a-sync-that-delivered-nothing-is-distinguished-from-a-healthy-one)
  - [UC-5 — A fresh install is read correctly](#uc-5--a-fresh-install-is-read-correctly)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Dependencies](#10-dependencies)
- [11. Assumptions](#11-assumptions)
- [12. Open Decisions](#12-open-decisions)
- [13. Risks](#13-risks)

<!-- /toc -->

Scoped addition to the [Analytics service](../../DESIGN.md) and the ingestion pipeline: what
an instance operator must be able to learn about the state of every connector, and what the
product may and may not claim about it. Realised by [DESIGN.md](DESIGN.md). Supersedes the
current Data health pane and the sketch in
[PR #2713](https://github.com/constructorfabric/insight/pull/2713), which is not merged.
## 1. Overview

### 1.1 Purpose

Give the instance operator one admin surface that answers, per connector: is it delivering
data, when did it last run, how did that run end, and — when it failed — where. The surface
reports recorded facts about ingestion runs; it never infers a verdict it cannot back.

### 1.2 Background / Problem Statement

The product today has no surface that reports connector state. The pane named *Data health*
counts schema-check statuses of metric definitions — a fact about the metric catalogue, not
about whether data arrives — and each of its counters overstates (custom metrics carry no
schema status by design, disabled definitions count as health, a schema-broken definition
with no observation counts twice).

The consequence is operational: a connector that quietly stops delivering looks identical to
one that was never configured, and the customer notices empty dashboards before the operator
notices the connector. Related backlog items describe the same pain from the alerting side
([#744](https://github.com/constructorfabric/insight/issues/744),
[#723](https://github.com/constructorfabric/insight/issues/723)); this PRD covers the
observation surface, not notifications.

Three failure classes are invisible today even to a diligent operator reading dashboards:

| Failure | Why it is invisible |
|---|---|
| Sync fails repeatedly | old data still renders; nothing states the pipeline is failing |
| Sync succeeds, transform fails | bronze is fresh, downstream layers silently stall |
| Sync reports records, storage gains none | the mover's self-report is the only account |

### 1.3 Goals (Business Outcomes)

- The operator learns that a connector is broken from the product, the same day it breaks —
  not from a user reporting an empty dashboard.
- Every ingestion run leaves a durable, queryable record, including runs that fail before
  the sync starts and runs started manually outside the pipeline.
- The page distinguishes *broken*, *empty*, *stopped*, and *never configured* — four states
  that are conflated today.
- No verdict is presented that the recorded facts cannot back; absence of knowledge renders
  as *unknown*, never as *healthy*.

### 1.4 Glossary

| Term | Meaning |
|---|---|
| **Connector** | one ingestion source type configured on the install; its raw data lands in a per-connector bronze schema |
| **Stream** | one relation within a connector's bronze schema |
| **Sync** | one execution of the data mover for one connector's connection |
| **Run** | one execution of the ingestion pipeline: sync followed by transform |
| **Transform** | the dbt step of a run that builds downstream layers from bronze |
| **Run ledger** | the append-only record of run and sync outcomes this PRD introduces |
| **Sweep** | the periodic reconciliation that copies sync outcomes from the mover's job history into the ledger |
| **Manual sync** | a sync started outside the pipeline (for example from the mover's own UI); no pipeline run claims its job identity |
| **Origin** | which writer recorded a ledger row: the pipeline itself or the sweep. Origin says who wrote the row, never how the sync was started |
| **Trigger** | how a sync was started: *pipeline* (a recorded run claims its job identity), *out-of-band* (corroborated by exact job identity that no run started it — concurrent timing is never evidence), or *unclaimed* (no reliable source; presented as unknown, never as manual) |
| **Configured connector** | a connector present in the reconcile controller's latest recorded snapshot of the set it manages; absence from the latest snapshot means no longer configured |
| **Honest unknown** | a value the system does not know rendered as unknown, never as zero or as healthy |
| **Physical rows** | rows present in storage parts; on a deduplicating engine this sizes a stream and does not count entities |

## 2. Actors

### 2.1 Human Actors

#### Instance operator

**ID**: `cpt-insightspec-connhealth-actor-operator`

**Role**: administers one Insight install; configures connectors and is accountable for data
arriving.
**Needs**: to see which connectors deliver, when each last ran, what a failed run failed on,
and whether a quiet connector is stopped, empty, or was never configured. The operator is
the only human actor permitted to read this surface.

### 2.2 System Actors

#### Ingestion pipeline

**ID**: `cpt-insightspec-connhealth-actor-pipeline`

**Role**: the orchestrated run (sync then transform) submitted by schedules, by connector
reconciliation, and by manual operator submission. First writer of the run ledger: it records
run boundaries and step outcomes as they happen.

#### Reconcile loop

**ID**: `cpt-insightspec-connhealth-actor-reconcile`

**Role**: the periodic controller that provisions connections from connector configuration.
Second writer of the run ledger: each tick it sweeps the mover's job history and records sync
outcomes the pipeline could not have written — manual syncs, and history from before the
ledger existed.

#### Data mover

**ID**: `cpt-insightspec-connhealth-actor-mover`

**Role**: executes syncs and keeps a bounded job history with per-job outcome and counters.
Source the sweep reads; never read on the page's request path.

#### Warehouse

**ID**: `cpt-insightspec-connhealth-actor-warehouse`

**Role**: stores bronze streams and the run ledger; its storage metadata provides per-stream
volume facts without reading data.

#### Analytics service

**ID**: `cpt-insightspec-connhealth-actor-analytics`

**Role**: serves the read surface to the portal from the ledger and storage metadata only.

#### Portal

**ID**: `cpt-insightspec-connhealth-actor-portal`

**Role**: renders the admin page: per-connector summary rows, expandable per-stream detail,
run history.

## 3. Operational Concept & Environment

Facts flow one way. At run time the pipeline records its own boundaries and outcomes into
the run ledger; on a fixed cadence the sweep reconciles the mover's job history into the same
ledger, covering runs the pipeline did not perform. At read time the analytics service
answers from the ledger plus warehouse storage metadata — no call leaves the warehouse on the
request path.

### 3.1 Module-Specific Environment Constraints

- **Bronze schemas are not tenant-partitioned**, so this read is instance-wide and gated on
  the operator role rather than scoped by tenant.
- **Bronze schemas exist for every connector the build knows**, configured or not; storage
  presence alone cannot distinguish *never configured* from *configured and broken*. The
  ledger provides that distinction.
- **Sync schedules live in the cluster's workflow layer**, not in the mover; the mover cannot
  say when the next sync is due, only what its past jobs did.
- **Freshness thresholds are declared per connector in transform-layer source definitions
  and have no runtime-readable form.** Until they do, the surface reports facts without
  fresh/stale verdicts.
- **Development compose stands run no mover and no workflow layer.** The surface is not
  supported there beyond not breaking: it renders its degraded state (see FR-13).

## 4. Scope

### 4.1 In Scope

- The run ledger: an append-only record of run and sync outcomes, written by the pipeline
  and the sweep.
- Sweep coverage of syncs started outside the pipeline, and one-time backfill of the ledger
  from the mover's retained job history.
- The delivery cross-check: records the mover reported paired with an independent count of
  storage rows attributable to that same sync.
- Recording of the currently configured connector set, so *never configured* is
  distinguishable from *configured and never ran*.
- The read surface: one admin endpoint and one portal page — per-connector summary,
  per-stream expansion, run history.
- Replacing the current Data health pane with this page.

### 4.2 Out of Scope

- **Alerting and notifications** — this PRD makes state observable;
  [#723](https://github.com/constructorfabric/insight/issues/723) owns pushing it.
- **Expectation and alerting on the connector set** — this surface records and shows what
  *is* configured (FR-15); deciding what *should* be configured for a given customer, and
  alerting on the gap, belongs to
  [#744](https://github.com/constructorfabric/insight/issues/744).
- **Freshness verdicts** (fresh / warn / stale) — deferred until declared thresholds have a
  runtime source.
- **Entity counts per stream** — physical row counts are in scope; deduplicated entity
  counts are a possible later drill-down.
- **Compose-stand support** beyond graceful degradation.
- **Fleet-level views across installs**
  ([#1816](https://github.com/constructorfabric/insight/issues/1816)).

## 5. Functional Requirements

### 5.1 Recording Runs

#### FR-1 — Every pipeline run leaves a terminal record

- [ ] `p1` - **ID**: `cpt-insightspec-connhealth-fr-terminal-record`

Every submitted run records its outcome and the step reached — including runs that fail
before the sync starts. A run that dies resolving its connection is a recorded failure naming
that step, not an absent row.

The recorder is deliberately best-effort: it must never fail the run it only observes, so a
recorder that cannot execute leaves that run without a terminal row. The page then shows no
terminal record rather than inventing an outcome, and nothing downstream reads the absence as
success.

#### FR-2 — Every sync outcome is recorded, whoever started it

- [ ] `p1` - **ID**: `cpt-insightspec-connhealth-fr-all-syncs-recorded`

Sync outcome, start time, duration, and moved-record counters are recorded for every
sync in the mover's job history — scheduled, reconcile-triggered, or started manually from
the mover's own UI. Every sync record carries the mover's job identity, and every
pipeline-recorded run claims the job it started by the same identity, so each sync is
attributable to the run that started it, corroborated as out-of-band, or reported unclaimed
when no reliable source remains. An out-of-band sync appears in the ledger no later than one
sweep interval after it completes.

#### FR-3 — Delivery is cross-checked, not self-reported

- [ ] `p2` - **ID**: `cpt-insightspec-connhealth-fr-delivery-crosscheck`

A sync's ledger record carries both what the mover reported moving and an independently
measured count of storage rows attributable to that same sync, so "reported records, storage
gained nothing" is a visible state rather than a trusted self-report. Where attribution is
impossible the pairing reports unknown — it never fabricates agreement, including on
backfilled history.

#### FR-4 — Transform outcome is recorded separately from sync outcome

- [ ] `p2` - **ID**: `cpt-insightspec-connhealth-fr-transform-outcome`

The transform step records its own outcome. A successful sync followed by a failed or absent
transform — downstream layers stalling while bronze stays fresh — is a distinct, visible
state.

#### FR-5 — History is bounded and survives the mover

- [ ] `p2` - **ID**: `cpt-insightspec-connhealth-fr-bounded-history`

Ledger rows are retained for a bounded, configurable period long enough to answer "when did
this break" and "how often does it fail". Retained history outlives connection deletion in
the mover.

#### FR-6 — First sweep backfills retained history

- [ ] `p2` - **ID**: `cpt-insightspec-connhealth-fr-backfill`

On an install where the ledger is new, the first sweep populates it from the mover's retained
job history, so the page shows run history from day one rather than after the first
post-deploy sync. Backfilled records carry only what the mover's history states; measurements
that can no longer be taken are recorded as unknown, never reconstructed.

#### FR-15 — The configured set is recorded

- [ ] `p2` - **ID**: `cpt-insightspec-connhealth-fr-configured-set`

The reconcile controller records, on its own cadence, a complete snapshot of the connector
set it currently manages — including the empty set, so removing the last connector is as
representable as removing any other. The latest snapshot is authoritative: removing a
connector's configuration is visible within one controller cadence, and absence needs no
retraction event.
A connector with a storage schema, no snapshot membership, and no runs is *never configured*;
one present in the snapshot with no runs is *configured and never ran*. The page
distinguishes the two.

### 5.2 Presenting State

#### FR-7 — One summary row per connector

- [ ] `p1` - **ID**: `cpt-insightspec-connhealth-fr-summary-row`

The page shows one row per connector: last run outcome and time, the failed step when the
run failed, moved-versus-landed counters, streams with data out of streams total, physical
rows, and size on disk.

#### FR-8 — A row expands to its streams

- [ ] `p2` - **ID**: `cpt-insightspec-connhealth-fr-stream-expansion`

Expanding a connector shows its streams with per-stream physical rows and share of streams
holding data, and when that picture was last observed. An empty stream inside an otherwise
delivering connector is the expansion's primary signal.

#### FR-9 — Facts, not verdicts

- [ ] `p1` - **ID**: `cpt-insightspec-connhealth-fr-facts-not-verdicts`

The surface asserts only what the ledger and storage metadata record. No fresh/stale
classification is shown while thresholds have no runtime source; a state the system cannot
know renders as *unknown*, never as healthy; row counts are labelled physical, never entity
counts.

#### FR-10 — Attention orders the page

- [ ] `p3` - **ID**: `cpt-insightspec-connhealth-fr-attention-order`

Failed and mismatched connectors sort above delivering ones; never-ran connectors sort last —
they are unfilled configuration, not failure.

#### FR-11 — Operator-only, instance-wide

- [ ] `p1` - **ID**: `cpt-insightspec-connhealth-fr-operator-only`

Only the instance operator may read the surface. The read is instance-wide; there is no
tenant-scoped variant. Any other caller is refused.

#### FR-12 — A sync's trigger is visible

- [ ] `p3` - **ID**: `cpt-insightspec-connhealth-fr-origin-visible`

Each recorded sync shows how it was started: claimed by a recorded pipeline run;
out-of-band — corroborated against the workflow layer's execution records, by exact job
identity, that no run started it; or unclaimed — no claim and no surviving record to
corroborate against, presented as unknown provenance and never asserted as manual.
Concurrent timing is never evidence: a manual sync running alongside a pipeline run must not
be attributed to it. The corroboration finding is recorded with the sync, so the page's
classification is reproducible from recorded facts alone. Because claims are recorded
best-effort, a lost claim must degrade to a late claim or to unclaimed — never to a false
out-of-band. An out-of-band sync is readable as such, including the case where it ran and no
transform followed. The recording writer (origin) is also visible, but origin alone is never
presented as the trigger.

### 5.3 Degradation

#### FR-13 — The page works with an empty ledger

- [ ] `p2` - **ID**: `cpt-insightspec-connhealth-fr-empty-ledger`

Before the first controller cadence, or on stands where nothing records, the page states
plainly that nothing has recorded an ingestion run yet. Where a connector is listed with no
observation behind it, its storage figures read as unknown rather than as zero. The page does
not error and does not imply health.

#### FR-14 — The read path depends on nothing external

- [ ] `p1` - **ID**: `cpt-insightspec-connhealth-fr-no-external-reads`

Serving the page requires only the recorded facts. No mover, cluster API, or other external
call is made on the request path, and the reader holds no access to raw connector data in any
form; unavailability upstream cannot degrade the page beyond the staleness of the last
recording.

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

#### NFR-1 — Reading is interactive

- [ ] `p2` - **ID**: `cpt-insightspec-connhealth-nfr-interactive-read`

Opening the page completes within an interactive budget on large installs, because the read
path touches only recorded facts — never raw connector data, and never a measurement taken
while the operator waits.

#### NFR-2 — Recording never breaks ingestion

- [ ] `p1` - **ID**: `cpt-insightspec-connhealth-nfr-recording-harmless`

A failure to write a ledger row must not fail the run being recorded, and sweep failures must
not disturb connector reconciliation. Observability is subordinate to the thing observed.

#### NFR-3 — The sweep is idempotent

- [ ] `p2` - **ID**: `cpt-insightspec-connhealth-nfr-idempotent-sweep`

Re-sweeping the same job history — after a crash, a restart, or overlap — does not change
what the page reports. Duplicate discovery is harmless.

#### NFR-4 — Storage is bounded

- [ ] `p3` - **ID**: `cpt-insightspec-connhealth-nfr-bounded-storage`

The ledger grows linearly with runs and is bounded by retention; it stays a small service
table and never enters customer-facing exports or metric relations.

### 6.2 NFR Exclusions

- **No real-time guarantee** — manual syncs become visible within one sweep interval, not
  instantly; an in-flight sync may be reported as running with sweep-interval staleness.
- **No alerting SLA** — see Out of Scope.
- **No completeness guarantee for pre-ledger history** — backfill reaches only as far as the
  mover's retained job history.

## 7. Public Library Interfaces

### 7.1 Public API Surface

#### Connector health read surface

**ID**: `cpt-insightspec-connhealth-interface-read-surface`

One operator-gated read surface on the analytics service returning the per-connector
summary, per-stream detail, and recent run history described in §5.2. Contract, shape, and
transport are specified in [DESIGN.md](DESIGN.md).

### 7.2 External Integration Contracts

#### Mover job history

**ID**: `cpt-insightspec-connhealth-contract-mover-history`

The sweep consumes the mover's job listing (outcome, timestamps, counters per job). This is
a background integration owned by the reconcile loop; the read surface never depends on it.

## 8. Use Cases

### UC-1 — A broken sync is triaged from the product

- [ ] `p1` - **ID**: `cpt-insightspec-connhealth-usecase-broken-sync`

A connector's nightly run fails. The operator opens the page, sees the connector sorted to
the top with its last run failed, the failed step named, and the run history showing when
failures began and how many attempts were made. The operator knows whether the failure is in
connection resolution, the sync itself, or the transform — before opening any logs.

### UC-2 — A silent stop is noticed

- [ ] `p1` - **ID**: `cpt-insightspec-connhealth-usecase-silent-stop`

A connector's schedule disappears (a suspended or deleted workflow). No new runs are
recorded. The page shows the connector's last recorded run receding into the past against
its own recorded cadence, so the operator sees "used to run daily, last ran N days ago" —
without the product claiming to know the intended schedule.

### UC-3 — A manual sync is accounted for

- [ ] `p2` - **ID**: `cpt-insightspec-connhealth-usecase-manual-sync`

Someone triggers a sync from the mover's own UI. Within one sweep interval the ledger holds
its outcome and counters; no pipeline run claims its job identity, corroboration against the
workflow layer's records confirms by job identity that no run started it, and the page
reports it as out-of-band
— a sync without a transform, downstream layers not rebuilt, which is exactly what a manual
mover-side sync does. A pipeline-started sync is never misreported as manual: its run claims
the job, and even a lost claim is repaired by corroboration or reported unclaimed rather than
asserted manual.

### UC-4 — A sync that delivered nothing is distinguished from a healthy one

- [ ] `p2` - **ID**: `cpt-insightspec-connhealth-usecase-zero-delivery`

Two connectors both report successful runs. One's ledger rows show the mover's moved-record
count and the storage rows attributed to that sync agreeing; the other's show records
reported while storage attributes nothing to the sync. The second is a visible mismatch —
misrouted writes or a broken destination — not a connector the page calls healthy.

### UC-5 — A fresh install is read correctly

- [ ] `p2` - **ID**: `cpt-insightspec-connhealth-usecase-fresh-install`

On a new install the operator configures a subset of connectors. The page lists configured
connectors with their first recorded runs, and shows the rest — schemas with no configuration
record and no runs — as never configured at the bottom, not as failures (FR-15). Before the first controller cadence completes, the page says no observations are available
yet rather than implying anything.

## 9. Acceptance Criteria

- [ ] A run that fails before its sync starts appears in the ledger with outcome and failed
      step (FR-1).
- [ ] A sync started from the mover's UI appears in the ledger within one sweep interval and
      is classified out-of-band; a pipeline-started sync never is, even when its best-effort
      claim was lost — it resolves to a late claim or to unclaimed (FR-2, FR-12).
- [ ] Every sync record pairs mover-reported counters with storage rows attributed to that
      sync, or reports the attribution as unknown (FR-3).
- [ ] A connector with a schema, no snapshot membership, and no runs renders as never
      configured; present in the snapshot with no runs, as configured and never ran; removed
      from configuration, it stops rendering configured within one controller cadence
      (FR-15).
- [ ] A failed transform after a successful sync is visible as its own state (FR-4).
- [ ] Deleting a connection in the mover does not remove already-recorded history (FR-5).
- [ ] On a ledger-less install the first sweep populates history from the mover's retained
      jobs (FR-6).
- [ ] The page renders one row per connector with the fields of FR-7, expandable per FR-8.
- [ ] No fresh/stale verdict appears anywhere; unknown states render as unknown (FR-9).
- [ ] A non-operator caller is refused; the response is instance-wide for the operator
      (FR-11).
- [ ] With the mover unreachable, the page still serves the last recorded facts, every
      trigger classification it shows is reproducible from them alone, and the reader holds
      no access to raw connector data (FR-12, FR-14).
- [ ] A ledger write failure does not fail the ingestion run (NFR-2); a repeated sweep does
      not change reported state (NFR-3).

## 10. Dependencies

- The ingestion pipeline's workflow layer, which must be able to record run boundaries
  (including abnormal termination) — realisation in [DESIGN.md](DESIGN.md).
- The reconcile loop, which already authenticates to the mover and runs on a periodic tick;
  the sweep extends it.
- The warehouse, which stores the ledger and serves storage metadata.
- Replacement target: the Manage-zone Data health pane in the portal.

## 11. Assumptions

- All non-manual sync paths submit the shared pipeline; a sync can bypass the pipeline only
  via the mover's own surfaces, which the sweep covers.
- The mover retains job history long enough for a useful backfill window; retention shorter
  than ledger retention limits backfill, not steady-state operation. Its listing is paged, so
  the window is the mover's retention rather than one page of it.
- The workflow layer can execute a step on run termination regardless of which step failed.
  This is a documented property of the workflow layer's exit handler, including for a
  failure in the first task, and the chart's render contract pins that every submitter
  reaches the pipeline in a way that carries the handler, so
  FR-1 needs no fallback.

## 12. Open Decisions

- ~~Ledger retention length~~ — settled at six months.
- ~~Per-stream moved-record counters from the mover's job detail~~ — deferred: the sweep
  consumes the public listing only, so nothing depends on an API that is not
  contract-stable across mover upgrades.
- ~~Naming of the replacement pane~~ — settled as Connector health.
- How far back the workflow layer's records can be trusted COMPLETE, which is the only
  window in which a missing claim means out-of-band. Implementation takes a configurable
  duration defaulting to a day; the records' actual retention is set by workflow garbage
  collection, which this change does not configure, so the two are not yet pinned to each
  other. Past the window a sync stays unclaimed, which is the safe answer — the exposure is
  a sync inside the window whose records were collected early being called out-of-band.

## 13. Risks

- ~~**Run-termination recording may not work as assumed**~~ — measured and working; see
  §11. What the same measurements found instead: a recorder placed after the work it records
  becomes the only thing its DAG's phase is read from, so a successful recording ERASED a
  failed sync and the run reported green. Mitigated by naming the real work as a phase target
  and by a rendered-chart test that fails if any DAG is ever assessed over recorders alone.
- **The sweep reads the mover's API across versions.** Mitigation: consume only its stable
  public listing for outcomes; treat any richer detail as optional enrichment that may
  degrade without degrading the page (FR-14 keeps the read path independent).
- **Self-report bias**: the ledger is written by the pipeline about itself. Mitigation:
  FR-3's storage cross-check pairs every self-report with an independent measurement. The
  pairing completes only once the sweep has supplied the mover's counters, so a mismatch is
  detectable within one controller cadence rather than immediately; until then each half
  reports itself as unknown rather than as zero.
- **Two writers can race** (sweep and pipeline recording the same sync). Mitigation: NFR-3
  makes duplicates harmless by construction; reads resolve per job identity, then by claim
  precedence.
