---
status: accepted
date: 2026-09-28
---

# ADR-0009: BullMQ Schedules the Alert Checks

<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-insightspec-v3-adr-bullmq-schedules-alert-checks`

## Context and Problem Statement

A metric alert checks a stored metric on an interval nobody is waiting on,
and owes a notification when the check finds the condition met. The checks
have to survive a restart, run once at a time per rule, recover when the
process that was running one dies, and never be replayed for the intervals
a restart missed. The service has no background work of this kind yet, and
this is the first of it: whatever runs alert checks is the shape every later
scheduled job in Insight will take.

Four ways to run that work were weighed against the service as it stands: a
MariaDB definition store reached through SeaORM, a Redis every deployment
already runs for sessions, and no other stateful component.

## Decision Drivers

* A check must run once per interval per rule, on one worker, and again
  after a crash without an operator touching anything.
* A restart must not replay the checks it missed.
* The library must build against the workspace as it is: one Redis driver,
  one SQL driver, no second pool.
* The same mechanism must serve the next background job without a redesign.

## Considered Options

* **Apalis 0.7 on MariaDB** — the library named in the first draft of the
  design. Its MySQL backend needs a second `sqlx` major beside SeaORM's; its
  schema migration names a collation older MariaDB releases refuse; and a
  failed job is handed to every polling worker at once until its attempts
  run out, because the claim update guards a status the retry path never
  has. Measured, not read: one always-failing job with four attempts ran
  twenty-four times across two workers, nine of them concurrently.
* **Apalis 1.0 release candidates on Redis** — no driver conflict. The
  per-task attempt limit is ignored in favour of a hardcoded twenty-five,
  and a worker that dies mid-job leaves a key that every later worker on
  the queue trips over at its first heartbeat; the queue stays unusable
  until the keys are deleted by hand.
* **A claim loop of our own on MariaDB** — `FOR UPDATE SKIP LOCKED`, a
  lease, a revision fence, a tokio task. Right for one feature, and the
  smallest change; but every later job would re-implement retries,
  stalled-work recovery and scheduling from scratch.
* **A durable-execution platform** — a new stateful component in every
  environment, invoking this service rather than the other way round, under
  a licence the project has already declined once for another runtime.
* **BullMQ through its official Rust port** — the same Lua scripts and Redis
  data model as the Node library: atomic state moves, lock renewal, stalled
  detection, retries with backoff, delayed jobs, deduplication by job id,
  and a scheduler that repeats a job on an interval and skips the iterations
  a slow run or a restart missed. Every one of those was exercised against a
  local Redis before this was written, and each behaved as documented.
  **Chosen.**

## Decision Outcome

Alert checks are BullMQ jobs on the deployment's Redis. An enabled rule is
one BullMQ job scheduler, keyed by the rule, repeating every rule interval;
the job carries the rule's revision, and the store records a check only for
the revision it was scheduled at. Rules, what their checks found, and the
notifications they owe live in MariaDB; Redis holds nothing that cannot be
rebuilt from them, and at startup the schedule is made to say what the
rules say.

The published crate pins every dependency exactly, and one of those pins
cannot coexist with the toolkit's; the workspace carries the published
source with the same versions as caret requirements under
`[patch.crates-io]`, and drops the copy the day upstream relaxes the pins.

### Consequences

* Good: retries, lock renewal, stalled recovery, deduplication and interval
  scheduling come from the library, so the next scheduled job in Insight
  adds a queue and a handler and nothing else.
* Good: one Redis client, one SQL client, no new stateful component.
* Bad: Redis becomes durability-critical for the schedule. It must persist;
  a Redis that loses its data stops every check until the next startup
  reconciles the schedule from the rules.
* Bad: the Rust port is months old. The Lua underneath is not, but a defect
  in the wrapper is ours to report and work around until it is fixed.
* Bad: the vendored copy has to be refreshed by hand until upstream stops
  pinning.

### Confirmation

The store's live tests prove a check lands only at its revision and one
breach owes one notification; the schedule's live tests prove a rule is
scheduled once however often it is written and that a worker takes a
scheduled check and records it. Both run in CI against a real MariaDB and
Redis.

## Traceability

- **PRD**: [Metric Alerts](../alerts/PRD.md), `cpt-insightspec-v3-alerts-nfr-durability`.
- **DESIGN**: [Metric Alerts](../alerts/DESIGN.md), `cpt-insightspec-v3-alerts-constraint-bullmq`.
- **Supersedes**: the Apalis constraint in earlier drafts of that design.
