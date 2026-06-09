# Observability installation for Insight

The bundled observability stack — **L**oki (logs), **A**lloy (collector),
**G**rafana (UI) — the "logs first" slice of LGTM. It runs as its own Helm
releases, separate from the Insight umbrella, the same way Airbyte and Argo
do.

This directory is the **reference** for `observability.mode: bundled`. It
is consumed by both deployment paths:

- **Local development** — `deploy/scripts/install-observability.sh` against
  the local cluster in the `insight` namespace.
- **Cluster deployment** — the private `infra/insight-gitops` repository
  drives the same charts from its `system/{loki,alloy,grafana}/values.yaml`
  overlays onto the `insight-infra` namespace as part of the L2 system layer.

## This is OPTIONAL — it depends on the mode

Insight services **always** emit structured JSON to stdout (and OTLP when a
collector is configured). That is the product contract and ships in the
umbrella chart (`observability.*`). WHERE logs/traces are collected is the
host cluster's choice:

| `observability.mode` | This stack? | Who collects |
|---|---|---|
| `bundled` | **install it** (this dir) | Insight-provided Loki/Grafana |
| `external` | do NOT install | the customer's OTLP collector (set `observability.otlp.endpoint`) |
| `none` | do NOT install | the host cluster's own node agent scrapes stdout |

Pick `external`/`none` when the target cluster already runs observability —
do not stand up a second collector that duplicates theirs.

## Pinned versions

| Path | Chart versions | Source of truth |
|---|---|---|
| `install-observability.sh` | `loki`/`alloy`/`grafana` (placeholders) | `deploy/scripts/install-observability.sh` |
| `infra/insight-gitops` | `LOKI_VERSION` / `ALLOY_VERSION` / `GRAFANA_VERSION` | `Makefile` in the gitops repo |

> Versions are placeholders until verified — run
> `helm search repo grafana/<chart> --versions` and pin deliberately.

## How services reach it

After install, point the umbrella at the in-cluster collector:

```yaml
observability:
  mode: bundled
  otlp:
    endpoint: http://alloy.<namespace>.svc.cluster.local:4317
```

The endpoint is published into the `{release}-platform` ConfigMap as
`OTEL_EXPORTER_OTLP_ENDPOINT`, so every service picks it up via `envFrom`
with no per-service wiring.

## Scope

Dev / single-tenant baseline: Loki SingleBinary on node-local filesystem,
no HA, no auth, 7-day retention. For production move Loki to object storage
(S3/GCS) with the read/write/backend split, seal a Grafana admin Secret, and
turn on multi-tenancy if one Loki ever serves more than one install. See the
per-file headers and `infra/insight-gitops:system/*/values.yaml` overlays.

## First logs

```
kubectl -n <ns> port-forward svc/grafana 3000:80
# Explore → Loki:
{namespace="airbyte"}            # raw Airbyte sync logs
{component=~".*dbt.*"}           # dbt run output
{component="reconcile-loop"}     # reconcile ticks
```
