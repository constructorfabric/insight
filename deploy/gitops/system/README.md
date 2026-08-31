# L2 — System Layer

Shared infrastructure services that live in the infra namespace
(inventory `namespaces.infra`, `insight-infra` by default), **one Helm
release per service**. `make system ENV=<env>`
chains every service whose `inventory.system.<svc>` toggle is true;
per-service `make system-<svc>` targets remain for one-offs and
rotation. The toggles exist because each cluster picks which services
it self-hosts vs. swaps for managed external endpoints (RDS, MSK,
Confluent Cloud, S3, …) or another team's infra.

See the top-level [`README.md`](../README.md) for the L0 / L2 / L3 layer
model and full workflow.

## Services

| Directory | Chart | Helm release (in the infra namespace) | Needs a Secret? |
|-----------|-------|---------------------------------|-----------------|
| `mariadb/` | `oci://registry-1.docker.io/bitnamicharts/mariadb` | `mariadb` | yes → [SECRETS.md](mariadb/SECRETS.md) |
| `clickhouse/` | `oci://registry-1.docker.io/bitnamicharts/clickhouse` | `clickhouse` | yes → [SECRETS.md](clickhouse/SECRETS.md) |
| `redis/` | `oci://registry-1.docker.io/bitnamicharts/redis` | `redis` | yes → [SECRETS.md](redis/SECRETS.md) |
| `redpanda/` | `redpanda/redpanda` | `redpanda` | not in baseline (TLS/SASL off); per-env overlay may add |
| `redpanda-console/` | `redpanda/console` | `redpanda-console` | not in baseline |
| `airbyte/` | `airbyte/airbyte` | `airbyte` | not in baseline (uses embedded Postgres+MinIO); prod overlay needs S3 creds |
| `argo-workflows/` | `argo/argo-workflows` | `argo-workflows` | not in baseline |
| `victoriametrics/` | `vm/victoria-metrics-single` | `victoriametrics` | not in baseline |
| `loki/` | `grafana-community/loki` | `loki` | not in baseline (single-tenant, no auth) |
| `tempo/` | `grafana-community/tempo` | `tempo` | not in baseline |
| `alloy/` | `grafana/alloy` | `alloy` | not in baseline |
| `kube-state-metrics/` | `prometheus-community/kube-state-metrics` | `kube-state-metrics` | not in baseline |
| `alloy-metrics/` | `grafana/alloy` | `alloy-metrics` | not in baseline |
| `grafana/` | `grafana-community/grafana` | `grafana` | not in baseline (chart auto-gens admin pw; per-env overlay may seal `grafana-creds`); ClickHouse datasource needs `grafana-clickhouse-creds` (see grafana/values.yaml, #2888) |

### Observability (victoriametrics / loki / tempo / alloy / kube-state-metrics / alloy-metrics / grafana)

These are the bundled observability stack: VictoriaMetrics stores metrics
(PromQL-compatible, remote-write receiver at `/api/v1/write`), Loki stores
logs, Tempo stores traces (OTLP ingest on `tempo:4317`/`4318`), Alloy
collects — it tails pod stdout to Loki and accepts OTLP from the services
on `alloy:4317` (gRPC) / `alloy:4318` (HTTP), remote-writing the metrics
to VictoriaMetrics and forwarding the traces to Tempo — and Grafana
serves all three — provisioned as the fixed-uid
datasources `vm`, `loki` and `tempo` so dashboards reference them portably;
a `trace_id` in a JSON log line links to the trace via Loki's
`derivedFields`.

**Infra metrics (kube-state-metrics / alloy-metrics).** Service health
without touching the services: `alloy-metrics` is a second Alloy release
(single-replica Deployment — every target is cluster-wide, so a DaemonSet
would scrape each once per node) that pull-scrapes the kubelet's cAdvisor
(per-container CPU / memory / network) and the `kube-state-metrics` release
(pod readiness, container restarts, OOMKills, deployment status),
remote-writing both into VictoriaMetrics. It also scrapes ClickHouse's
built-in Prometheus endpoint (`metrics.enabled` in
`system/clickhouse/values.yaml`, port 8001, #2888). Enable them together
with `inventory.system.{kubeStateMetrics,alloyMetrics}`; they need
`victoriametrics` on to have somewhere to write.

**ClickHouse SQL from Grafana (#2888).** Grafana also carries a
`clickhouse` fixed-uid datasource for direct SQL over the native protocol,
authenticating as the SELECT-only `grafana` user the app deploy hook
provisions (`provision-grafana-access.sh`). It needs the same password in
two Secrets: `insight-db-creds` key `clickhouse-grafana-password` (ns
`insight`, consumed by the hook) and `grafana-clickhouse-creds` key
`CLICKHOUSE_GRAFANA_PASSWORD` (ns `insight-infra`, injected into Grafana's
environment). Both optional — without them the role is still provisioned
and only the datasource stays dark.

Two independent decisions, mirroring the
managed-vs-bundled choice for the data stores above:

1. **Install the bundled stack?** — the
   `inventory.system.{victoriametrics,loki,tempo,alloy,kubeStateMetrics,alloyMetrics,grafana}`
   toggles.
   On = self-host the stack in `insight-infra`. Off = don't (the cluster
   already runs observability, or stdout is enough).
2. **Where do services export?** — the umbrella's `observability.otlp.endpoint`
   (`environments/<env>/values.yaml`). Point it at this stack's Alloy when the
   toggles are on; at your own collector for an external one; leave it empty
   for stdout-only.

Services ALWAYS log structured JSON to stdout regardless — that is the
product contract; the endpoint only decides where (if anywhere) Insight also
exports OTLP.

**Dashboards.** `system/grafana/values.yaml` provisions two log-based
dashboards into the "Insight" folder: HTTP (request rate by status class,
latency percentiles per route — built on the api-gateway access log) and
Ingestion & deploys (reconcile / airbyte-sync / dbt pod logs, deploy hook
pods, shipped helm output). Deploy markers come from the
`insight-post-upgrade` hook pod; `make deploy-app` additionally ships its
helm log to Loki via `scripts/push-deploy-log.sh` (best-effort).

**Access (follow-up: auth).** The baseline Grafana ships with no ingress and
no SSO — reach it via port-forward (commands assume the default
`namespaces.infra: insight-infra`; swap in your inventory's value):

```shell
kubectl -n insight-infra port-forward svc/grafana 3000:80
# admin password:
kubectl -n insight-infra get secret grafana -o jsonpath='{.data.admin-password}' | base64 -d
# Explore → Loki:
{namespace="insight"}            # service logs
{component="reconcile-loop"}     # reconcile ticks
# Explore → VictoriaMetrics (PromQL; empty until a collector remote-writes —
# alloy-metrics fills it with per-pod series when enabled):
up
container_memory_working_set_bytes{namespace="insight"}
kube_pod_container_status_restarts_total{namespace="insight"}
```

Putting Grafana behind auth is a tracked follow-up: seal a `grafana-creds`
Secret (the optional-secrets helper applies it), then per-env ingress + OIDC
SSO via the existing `insight-oidc` app.

## Values layout

```
system/<svc>/values.yaml                            # shared base — applied to every env
environments/<env>/<svc>-values.yaml                # per-env overlay — created only when an env diverges
```

Both are passed to `helm upgrade --install` in that order. Missing
overlay file = base values used as-is.

### Cross-service hostnames

Cross-service references are written as `${NS_*}` placeholders, and
neither file reaches helm verbatim: `scripts/render-system-values.sh`
first resolves the `${NS_*}` placeholders that cross-service references
use (`http://loki.${NS_LOKI}.svc.cluster.local:3100`, …), writing the
rendered copies to `.deploy/system-values/` — inspect them there after a
run. Two variables cover every cross-service reference:

| Variable | Locates | Consumers |
|----------|---------|-----------|
| `NS_INFRA` | the data stores (`clickhouse`, `redpanda`) | alloy-metrics, grafana, redpanda-console |
| `NS_MONITORING` | the observability unit (`loki`, `tempo`, `victoriametrics`, `kube-state-metrics`) | alloy, alloy-metrics, grafana |

`NS_MONITORING` defaults to `NS_INFRA` — the layout every environment
here uses — and exists because the observability stack is the one unit
an environment plausibly hosts apart from the data stores.

Service *names* stay literal on purpose — `fullnameOverride` in each
producer's values pins them, so the name is the contract and only the
namespace moves. Substitution replaces only these two exact braced
tokens: every other `$` construct — Grafana
provisioning `$VAR` / `$$VAR` escapes, `$__auto` in dashboards,
`${…}`-shaped strings in Alloy configs — passes through byte-identical,
so overlays that carry full Alloy configs or restated datasource lists
can keep using the placeholders too.

## Secret layout

```
environments/<env>/sealed-secrets/<infra-namespace>/<svc>-creds-sealedsecret.yaml
```

(`<infra-namespace>` is the inventory's `namespaces.infra` —
`insight-infra` by default; the Makefile resolves the directory from it.)

Files are sealed against the cluster's sealed-secrets-controller public
cert (`environments/<env>/pub-cert.pem`). Source of truth for the
cleartext is your chosen password manager — `make seal-secret` shells
out to `scripts/secret-fetch.sh` with the resource name
`insight-<env>-<svc>-creds` and pipes the result to `kubeseal`. The
shipped stub reads from a local YAML file; replace it with your own
backend (Vault, 1Password, Bitwarden, AWS Secrets Manager, Passbolt, …).
See each service's `SECRETS.md` for the exact key shape and a paste-able
payload.

`make system-<svc>` enforces: if the Bitnami chart's
`auth.existingSecret` references a Secret that has no sealed manifest
in the repo, the target fails with the exact `make seal-secret …`
command to run and a pointer at this directory. No silent installs
against missing creds.

## Switching to a managed external endpoint

A cluster that uses a managed service (RDS for MariaDB, MSK for
Redpanda, Confluent Cloud, S3, …) simply does NOT run the corresponding
`make system-<svc>` target. Instead, the app layer (umbrella) values
point at the external host:

```yaml
# environments/<env>/values.yaml
mariadb:
  deploy: false
  host: <rds-endpoint>.<region>.rds.amazonaws.com
  port: 3306
  database: insight
  username: insight
  passwordSecret:
    name: insight-db-creds   # still a sealed-secret, in the `insight` namespace
    key:  mariadb-password
```

The umbrella's `mariadb.deploy: false` toggle skips the subchart; the
app reaches the managed endpoint at the host/port supplied.
