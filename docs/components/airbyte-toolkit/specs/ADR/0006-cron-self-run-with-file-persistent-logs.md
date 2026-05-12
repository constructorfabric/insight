---
status: accepted
date: 2026-05-05
decision-makers: platform-engineering
---

# ADR-0006: Cron Self-Run with File-Persistent Logs


<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Option A — Run reconcile from CI on a scheduled GitHub Action](#option-a--run-reconcile-from-ci-on-a-scheduled-github-action)
  - [Option B — Argo CronWorkflow + PVC log file](#option-b--argo-cronworkflow--pvc-log-file)
  - [Option C — Native Kubernetes CronJob](#option-c--native-kubernetes-cronjob)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-insightspec-adr-cron-self-run-with-file-persistent-logs`

## Context and Problem Statement

After PR #281 retired the Kestra orchestrator, the reconcile loop has no autonomous trigger. We need (a) a cluster-native scheduler that runs `bash src/ingestion/reconcile-connectors/main.sh` on a fixed cadence and (b) durable logs that survive pod restarts and can be inspected without `kubectl logs --previous` gymnastics. We also want noise-free observability: a `*/15` cron driving 1000+ runs per quarter must not flood stdout with idle "nothing to do" lines, and must not balloon a logfile on quiet days.

How do we trigger reconcile autonomously inside the cluster, where do its logs live, and how do we keep idle ticks silent?

## Decision Drivers

- **Cluster autonomy**: reconcile must continue running when CI is offline or air-gapped.
- **Operator-visible liveness**: `kubectl logs` must show *something* on every tick, even quiet ones.
- **Durable change history**: a definition or Secret change made on Friday must be discoverable on Monday — pod logs alone are insufficient.
- **Quiet-run discipline**: 1000+ runs/quarter must NOT inflate the log file when nothing has changed.
- **Schedule mutability**: operators must be able to dial frequency up/down via a single Helm value.
- **Reuse Argo**: the rest of ingestion already runs on Argo; introducing a second scheduler is friction.

## Considered Options

- **Option A** — Run reconcile from CI on a scheduled GitHub Action.
- **Option B** — Argo CronWorkflow + PVC log file, with quiet-run policy (CHOSEN).
- **Option C** — Native Kubernetes `batch/v1` CronJob.

## Decision Outcome

Chosen option: **Option B — Argo CronWorkflow + PVC log file**.

**Justification**: an umbrella Helm chart renders an Argo `CronWorkflow` `insight-reconcile-loop` on schedule `{{ .Values.ingestion.reconcile.schedule | default "*/15 * * * *" }}` running `bash src/ingestion/reconcile-connectors/main.sh` in the toolbox image. A dedicated `ServiceAccount` with RBAC scoped to `secrets`/`onepassworditems`/`configmaps` (read), `workflows.argoproj.io`/`cronworkflows.argoproj.io` (CRUD), and Airbyte API access (in-cluster URL) drives the loop. Logs go to `/var/log/insight/reconcile-${YYYY-MM-DD}.log` on PVC `insight-reconcile-logs` (default 5Gi, override `ingestion.reconcile.logs.size`). The toolkit writes file lines ONLY on changes or errors; every run emits exactly one stdout summary line so `kubectl logs` shows liveness. Bootstrap on a fresh cluster fans out N parallel sync Workflows, with Airbyte itself queuing — no rate-limiting in our code.

### Consequences

- **Good**, because reconcile is autonomous and survives orchestrator changes.
- **Good**, because daily filename rotation + change/error-only writes make 1000-runs-per-quarter analysis tractable.
- **Good**, because one stdout summary line per run is enough for liveness alerts.
- **Good**, because schedule is a single Helm value — operators can dial it down for noisy environments.
- **Bad**, because the PVC must exist before the chart is installed; storage class is an environment-specific value.
- **Bad**, because retention is manual; on heavily-changing connectors operators must rotate-and-archive the logs themselves.
- **Bad**, because bootstrap of a new cluster fires N parallel sync Workflows; Airbyte queues these — surprise-able if cluster operators don't expect a burst.

### Confirmation

- `helm template … | grep -A4 schedule` shows the resolved schedule string and confirms the CronWorkflow object is created.
- Idempotency harness (Phase 18) runs the loop 100× on a quiet workspace and asserts `wc -l <logfile>` is unchanged from the pre-run snapshot.
- The same harness asserts 100 stdout summary lines are emitted (one per run).
- Integration test on a live cluster: cluster-install the chart → wait one cron tick → `kubectl get cronworkflow.argoproj.io insight-reconcile-loop` returns OK and the next tick produces a `Workflow`.

## Pros and Cons of the Options

### Option A — Run reconcile from CI on a scheduled GitHub Action

A GitHub Actions workflow with `schedule: cron: '*/15 * * * *'` invokes reconcile against the cluster.

- Good, because nothing to operate inside the cluster.
- Bad, because cluster autonomy is lost — CI outages stop reconcile.
- Bad, because secrets must be exfiltrated to CI to authenticate against Airbyte/Kubernetes.
- Bad, because the round-trip latency (GitHub runner → cluster) lengthens the feedback loop on cluster-local issues.
- Bad, because not viable for air-gapped deploys.

### Option B — Argo CronWorkflow + PVC log file

Umbrella Helm chart renders the CronWorkflow + PVC + ServiceAccount/RBAC. `lib/log.sh` writes to a daily-rotated file and emits one stdout summary line per run.

- Good, because cluster-autonomous; logs survive pod restarts.
- Good, because daily filename rotation makes mtime triage easy.
- Good, because quiet-run policy keeps signal-to-noise high.
- Good, because uses existing Argo machinery (no new scheduler topology).
- Neutral, because PVC lifecycle is now owned by the chart.
- Bad, because manual log cleanup (retention indefinite per the file-logs decision).
- Bad, because bootstrap submits N parallel sync Workflows on the first tick of a new cluster.

### Option C — Native Kubernetes CronJob

Replace Argo CronWorkflow with a vanilla `batch/v1` CronJob.

- Good, because one less CRD to think about in pure-Kubernetes terms.
- Bad, because lacks workflow primitives needed elsewhere (init-step name resolver, sub-workflow templates), forcing a hybrid scheduler topology.
- Bad, because not aligned with the rest of the ingestion stack which already runs on Argo (extra mental tax for operators).

## More Information

- A future iteration may export reconcile metrics to Prometheus; for now logs are the single observability surface.
- The ServiceAccount RBAC is the minimum surface — extending it (e.g., to `cluster-admin`) is explicitly out of scope.
- Optional integration with cluster log aggregators (Loki, fluent-bit) can read from the PVC; nothing in this ADR prevents it.
- Related decisions:
  - `cpt-insightspec-adr-connection-name-as-argo-identifier` (ADR-0005) — the per-connector CronWorkflow this ADR's loop manages stores `connection_name` for recreate resilience.
  - `cpt-insightspec-adr-auto-trigger-sync-on-data-change` (ADR-0008) — the per-tick reconcile may submit one-shot Workflows; this ADR provides the cron pod that performs that submission.

## Traceability

- **PRD**: [PRD.md](../PRD.md)
- **DESIGN**: [DESIGN.md](../DESIGN.md) §3.14, §3.15

This decision directly addresses:

- `cpt-insightspec-fr-cron-self-run` — the FR.
- `cpt-insightspec-fr-file-persistent-logs` — the durable-log half of the decision.
- `cpt-insightspec-fr-leak-free-loop` — the quiet-run policy that makes 1000×-safe possible.
- `cpt-insightspec-component-reconcile-cronworkflow` — the cluster-level CronWorkflow component.
- `cpt-insightspec-component-reconcile-file-logger` — the daily-rotated logger.
