---
status: accepted
date: 2026-05-08
decision-makers: platform-engineering
---

# ADR-0012: Bronze ClickHouse destination is owned by reconcile

<!-- toc -->
<!-- /toc -->

**ID**: `cpt-insightspec-adr-destination-owned-by-reconcile`

## Context and Problem Statement

A fresh Airbyte instance has zero destinations. The reconcile loop, when bootstrapping connections from secrets, must point them at a Bronze sink in ClickHouse. Two earlier options were on the table:

1. Operator pre-creates the destination via Airbyte UI / Terraform and pastes its UUID into Helm value `ingestion.reconcile.destinationId`.
2. A bootstrap Job in the chart calls Airbyte API at install time to create the destination, then writes its UUID into a ConfigMap that reconcile reads.

Both add a non-secret config knob the operator has to know about, conflict with the project rule that the operator only configures secrets, and surface an Airbyte UUID in chart state.

## Decision Drivers

- **Operator simplicity** — operator only configures secrets, never UUIDs.
- **Self-healing** — destination missing or recreated → reconcile re-creates it on next run, no manual recovery.
- **Single source of truth** — destination configuration lives next to the rest of reconcile state (chart values + creds secret).
- **Clean failure on misconfiguration** — wrong host / port / credentials produce a real Airbyte error, not silent drift.

## Considered Options

- **Option A** — `destinationId` Helm value, fail-fast on empty.
- **Option B** — Bootstrap Job + ConfigMap-based UUID handoff.
- **Option C** — Reconcile resolves-or-creates destination by name (CHOSEN).

## Decision Outcome

Chosen option: **Option C — reconcile owns the Bronze destination**.

**Justification**: reconcile already runs on a cron and already has the auth context to talk to Airbyte. Owning the destination from the same loop that owns sources / connections / sync triggers gives a single self-healing surface. Resolution is by name (`clickhouse-bronze` by default, override via `ingestion.reconcile.destinationName`), so there is no UUID for the operator to memorise. The connection configuration is built from chart values + the cluster's existing `clickhouse.passwordSecret`, none of which are new operator-facing knobs.

The legacy env `RECONCILE_DESTINATION_ID` remains a soft override: if set, reconcile uses it verbatim and skips lookup. This keeps backward compatibility with manually pre-created destinations while letting fresh installs not need it.

### Consequences

- **Good**, because operator's mental model becomes "I configure secrets and tenant id, the loop owns everything else".
- **Good**, because moving Airbyte / re-deploying with a fresh DB does not require a manual re-paste.
- **Good**, because the destination is recreated automatically if accidentally deleted in Airbyte UI.
- **Bad**, because reconcile now needs ClickHouse host / port / db / username / password env vars (mounted by the chart). Mitigation: every other ingestion component (`dbt-run`, `tt-enrich-jira-run`) already reads the same set, so chart wiring is consistent.

### Confirmation

- Helm install on a cluster with empty Airbyte → reconcile creates `clickhouse-bronze` destination on its first run; subsequent runs report `noop` for the destination layer.
- Manual delete of the destination in Airbyte UI → next reconcile recreates it; existing connections (orphaned by the delete) continue to point at the new id since reconcile re-resolves on each loop.
- `ab_ensure_destination` is idempotent at the API level: a second call with the same name short-circuits on `destinations/list` match.

## Pros and Cons of the Options

### Option A — `destinationId` Helm value

- Good, because trivially understood.
- Bad, because operator must run a one-shot Airbyte API call before `helm install` and paste a UUID. Violates "secrets only".
- Bad, because re-deploying Airbyte requires re-pasting the new UUID.

### Option B — Bootstrap Job

- Good, because operator-invisible.
- Bad, because Job ordering vs reconcile is fragile; a chart upgrade that recreates the Job would re-create or duplicate the destination.
- Bad, because UUID handoff via ConfigMap leaks Airbyte state into chart state.

### Option C — Reconcile-owned

- Good, because owner of the operation is the loop that already runs on cron.
- Good, because failure mode is "next loop fixes it".
- Neutral, because adds a few env vars to reconcile-cron pod (already pulls the same data for other ingestion pods).

## More Information

- Implementation: `ab_ensure_destination` + `ab_destination_definition_id_by_name` in [airbyte.sh](../../../../src/ingestion/reconcile-connectors/lib/airbyte.sh); `reconcile_resolve_destination_id` in [reconcile.sh](../../../../src/ingestion/reconcile-connectors/lib/reconcile.sh).
- Helm wiring: `RECONCILE_DEST_CLICKHOUSE_*` env vars in [reconcile-cron.yaml](../../../../charts/insight/templates/ingestion/reconcile-cron.yaml).
- Related decisions:
  - `cpt-insightspec-adr-airbyte-workspace-as-namespace` (ADR-0009) — workspace where the destination lives.
  - `cpt-insightspec-adr-version-driven-reconcile` (ADR-0001) — overall reconcile loop that owns this resource.

## Traceability

- **PRD**: [PRD.md](../PRD.md)
- **DESIGN**: [DESIGN.md](../DESIGN.md)
- **FEATURE**: [feature-reconcile/FEATURE.md](../feature-reconcile/FEATURE.md)
