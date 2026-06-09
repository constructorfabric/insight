# Observability stack for Insight

The bundled observability stack — **L**oki (logs), **A**lloy (collector),
**G**rafana (UI) — the "logs first" slice of LGTM. It runs as its own Helm
releases, separate from the Insight umbrella, the same way Airbyte and Argo
do.

This directory holds the **reference values** for those three charts. The
files here are the source consumed by the gitops system layer
(`infra/insight-gitops:system/{loki,alloy,grafana}/values.yaml`), which is
what actually installs the stack onto a cluster — there is no standalone
install script (install is `make`-driven in gitops, mirrored to the public
gitops-sample). For local development the stack is not required at all.

## This is OPTIONAL — driven by `observability.otlp.endpoint`

Insight services **always** emit structured JSON to stdout — that is the
product contract, shipped in the umbrella chart (`observability.*`). Whether
they also export OTLP, and where, is driven purely by the endpoint:

| `observability.otlp.endpoint` | This stack? | Who collects |
|---|---|---|
| set to this stack's Alloy | **install it** | Insight-provided Loki/Grafana (gitops `system/` toggles) |
| set to the customer's collector | do NOT install | the customer's OTLP collector / Datadog / Splunk |
| empty | do NOT install | the host cluster's own node agent scrapes stdout |

Leave the endpoint empty (or point it at an existing collector) when the
target cluster already runs observability — don't stand up a second
collector that duplicates theirs. Whether the bundled stack gets installed
is decided by the gitops `inventory.system.{loki,alloy,grafana}` toggles.

## Install

Cluster installs go through gitops (`make system-loki`, `system-alloy`,
`system-grafana`, or the chained `make system`). The public gitops-sample
documents the same for outside operators.

## Pinned versions

Chart versions live in the gitops `Makefile` (`LOKI_VERSION` /
`ALLOY_VERSION` / `GRAFANA_VERSION`) — single source of truth.

> Versions are placeholders until verified — run
> `helm search repo grafana/<chart> --versions` and pin deliberately.

## How services reach it

Point the umbrella at the in-cluster collector:

```yaml
observability:
  otlp:
    endpoint: http://alloy.<namespace>.svc.cluster.local:4317
```

The endpoint is published into the `{release}-platform` ConfigMap as
`OTEL_EXPORTER_OTLP_ENDPOINT`, so every service picks it up via `envFrom`
with no per-service wiring. Empty endpoint → no OTEL_* vars → stdout only.

## Access (auth)

The bundled Grafana ships with **no ingress and no auth** — reach it via
`kubectl port-forward` for now. Authn/SSO (via the existing `insight-oidc`
app) and an ingress are a deliberate follow-up, added per-env when needed.

## Scope

Dev / single-tenant baseline: Loki SingleBinary on node-local filesystem,
no HA, no auth, 7-day retention. For production move Loki to object storage
(S3/GCS) with the read/write/backend split, seal a Grafana admin Secret, and
turn on multi-tenancy if one Loki ever serves more than one install. See the
per-file headers and `infra/insight-gitops:system/*/values.yaml` overlays.

## First logs

```shell
kubectl -n <ns> port-forward svc/grafana 3000:80
# Explore → Loki:
{namespace="airbyte"}            # raw Airbyte sync logs
{component=~".*dbt.*"}           # dbt run output
{component="reconcile-loop"}     # reconcile ticks
```
