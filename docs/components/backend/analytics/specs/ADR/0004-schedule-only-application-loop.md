---
status: accepted
date: 2026-11-05
---

# ADR-0004: Apply connector schedules with a one-minute loop

## Context

Schedule requests should reach existing connector CronWorkflows promptly. Full connector
reconciliation also manages Airbyte definitions, sources and connections, removes retired
instances and records sync history. Running all of that for a schedule edit adds unrelated
work and failure paths.

Analytics stores revisioned requests. Ingestion reads their complete snapshot through
`connector_schedule_requests_v1`, resolves request → annotation → descriptor → default,
and writes an acknowledgement record only after verifying the applied CronWorkflow.

## Decision

Run a schedule-only ingestion CronWorkflow every minute. Keep the full reconcile cadence.
Both workflows acquire the release-scoped Argo `-reconcile` mutex before their pods read
inputs. Manual full passes use the same mutex-bearing CronWorkflow, for example through
`argo submit --from cronwf/<release>-reconcile-loop`; direct pod execution does not provide
that serialization.

Each tick reads requests, instance Secrets, descriptors, managed connector CronWorkflows
and acknowledgement records before making changes. It reuses instance discovery, schedule
resolution, rendering and acknowledgement verification from reconciliation. A failed read
fails the tick without applying defaults or changing schedules.

The loop patches only schedules, timezone and schedule-origin on existing CronWorkflows,
fenced by UID and resourceVersion. It never creates or deletes them, contacts Airbyte,
sweeps sync history or submits a full pass. Missing CronWorkflows receive an attempt error:
the next full pass creates them. Matching schedules and acknowledgement records need no
write. An unchanged schedule with a missing or outdated acknowledgement is verified and
acknowledged without a patch.

After a patch, ingestion reads the same object back and verifies identity and schedule
before advancing the acknowledgement record. A failure records only the attempt and
preserves the previous successful observation. The acknowledged revision comes from the
tick's snapshot, so a newer concurrent save stays pending. A clear applies the current
deployment fallback at its own revision.

## Options considered

| Option | Assessment |
| --- | --- |
| Schedule-only loop | Chosen: bounded work, durable requests and reuse of ingestion's ownership and resolution rules. |
| Faster full pass | Repeats Airbyte reconciliation and sync history work for schedule changes; no measurements justify the extra work. |
| Polling trigger submitting full passes | Over-built: ETags, burst settling and queued submissions repeat work already represented by durable requests. An in-memory edit queue loses edits across a restart. |
| Analytics calling Argo | Gives analytics workflow credentials and couples a request save to orchestration availability. |
| Argo Events | Adds an event transport and delivery/retry machinery for state that can be read once a minute. |
| Analytics patching CronWorkflows | Moves Kubernetes credentials and ingestion's resolver/rendering responsibilities into analytics. |

## Consequences

Idle application latency is 0–60 seconds plus pod startup and application time. Frontend
polling adds display latency.

Serialization with a running full pass can delay application up to that pass's
1,800-second deadline, plus startup and application time. Pods read inputs after acquiring
the mutex, so a waiting tick does not apply a snapshot captured before the full pass.

The operating cost is one short pod per minute. `Forbid` prevents overlapping loop ticks;
bounded job history, TTL and pod garbage collection limit retained objects. Durable
requests and readback acknowledgements make retries safe after a patch or database write
failure, including a restart between those steps.
